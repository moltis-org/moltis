// Pure functions are used by the WASM Guest impl; allow dead_code on non-wasm targets.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

#[cfg(target_arch = "wasm32")]
wit_bindgen::generate!({
    path: "../../../wit",
    world: "pure-tool",
});

use {
    anyhow::{Result, bail},
    serde_json::{Value, json},
};

const MAX_EXPRESSION_CHARS: usize = 512;
const MAX_TOKENS: usize = 256;
const MAX_AST_DEPTH: usize = 64;
const MAX_OPERATIONS: usize = 512;
const MAX_ABS_EXPONENT: f64 = 1024.0;
const MAX_ABS_RESULT: f64 = 1.0e308;

#[cfg(target_arch = "wasm32")]
struct CalcComponent;

#[cfg(target_arch = "wasm32")]
impl Guest for CalcComponent {
    fn name() -> String {
        "calc".to_string()
    }

    fn description() -> String {
        "Evaluate arithmetic expressions safely. Supports +, -, *, /, %, ^, unary +/- and parentheses. \
         No variables, functions, or assignments."
            .to_string()
    }

    fn parameters_schema() -> String {
        json!({
            "type": "object",
            "required": ["expression"],
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "Arithmetic expression to evaluate (for example: (2 + 3) * 4, 15%4, 2^8)"
                },
                "expr": {
                    "type": "string",
                    "description": "Alias for expression"
                }
            }
        })
        .to_string()
    }

    fn execute(params_json: String) -> ToolResult {
        match execute_impl(&params_json) {
            Ok(value) => ToolResult::Ok(ToolValue::Json(value.to_string())),
            Err(error) => ToolResult::Err(ToolError {
                code: "invalid_input".to_string(),
                message: error.to_string(),
            }),
        }
    }
}

fn execute_impl(params_json: &str) -> Result<Value> {
    let params: Value = serde_json::from_str(params_json)?;
    let expression = params
        .get("expression")
        .or_else(|| params.get("expr"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing 'expression' parameter"))?;

    let (result, normalized_expr) = evaluate_expression(expression)?;
    Ok(json!({
        "result": result_to_json(result)?,
        "normalized_expr": normalized_expr
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Power,
}

impl Operator {
    fn precedence(self) -> u8 {
        match self {
            Self::Add | Self::Subtract => 10,
            Self::Multiply | Self::Divide | Self::Modulo => 20,
            Self::Power => 30,
        }
    }

    fn is_right_associative(self) -> bool {
        matches!(self, Self::Power)
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Modulo => "%",
            Self::Power => "^",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number { value: f64, repr: String },
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
}

impl Token {
    fn repr(&self) -> String {
        match self {
            Self::Number { repr, .. } => repr.clone(),
            Self::Plus => "+".to_string(),
            Self::Minus => "-".to_string(),
            Self::Star => "*".to_string(),
            Self::Slash => "/".to_string(),
            Self::Percent => "%".to_string(),
            Self::Caret => "^".to_string(),
            Self::LParen => "(".to_string(),
            Self::RParen => ")".to_string(),
        }
    }
}

fn normalize_negative_zero(value: f64) -> f64 {
    if value.classify() == std::num::FpCategory::Zero {
        0.0
    } else {
        value
    }
}

fn is_zero(value: f64) -> bool {
    value.classify() == std::num::FpCategory::Zero
}

fn ensure_finite(value: f64, context: &str) -> Result<f64> {
    if !value.is_finite() || value.abs() > MAX_ABS_RESULT {
        bail!("{context} produced a non-finite result");
    }
    Ok(normalize_negative_zero(value))
}

fn parse_number_token(expression: &str, start: usize) -> Result<(Token, usize)> {
    let bytes = expression.as_bytes();
    let mut i = start;
    let mut saw_digit = false;

    if bytes.get(i) == Some(&b'.') {
        i += 1;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            saw_digit = true;
            i += 1;
        }
        if !saw_digit {
            bail!("invalid number literal at byte {start}");
        }
    } else {
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            saw_digit = true;
            i += 1;
        }
        if bytes.get(i) == Some(&b'.') {
            i += 1;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                saw_digit = true;
                i += 1;
            }
        }
    }

    if !saw_digit {
        bail!("invalid number literal at byte {start}");
    }

    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let exponent_marker = i;
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exponent_start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if exponent_start == i {
            bail!("invalid exponent in number literal at byte {exponent_marker}");
        }
    }

    let repr = expression[start..i].to_string();
    let value = repr
        .parse::<f64>()
        .map_err(|_| anyhow::anyhow!("invalid number literal `{repr}`"))?;
    let value = ensure_finite(value, "number literal")?;

    Ok((Token::Number { value, repr }, i))
}

fn tokenize(expression: &str) -> Result<Vec<Token>> {
    let bytes = expression.as_bytes();
    let mut i = 0usize;
    let mut tokens = Vec::new();

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => {
                i += 1;
            },
            b'0'..=b'9' | b'.' => {
                let (token, next) = parse_number_token(expression, i)?;
                tokens.push(token);
                i = next;
            },
            b'+' => {
                tokens.push(Token::Plus);
                i += 1;
            },
            b'-' => {
                tokens.push(Token::Minus);
                i += 1;
            },
            b'*' => {
                tokens.push(Token::Star);
                i += 1;
            },
            b'/' => {
                tokens.push(Token::Slash);
                i += 1;
            },
            b'%' => {
                tokens.push(Token::Percent);
                i += 1;
            },
            b'^' => {
                tokens.push(Token::Caret);
                i += 1;
            },
            b'(' => {
                tokens.push(Token::LParen);
                i += 1;
            },
            b')' => {
                tokens.push(Token::RParen);
                i += 1;
            },
            other => bail!("unsupported character `{}` at byte {}", other as char, i),
        }
    }

    if tokens.is_empty() {
        bail!("expression is empty");
    }
    if tokens.len() > MAX_TOKENS {
        bail!("expression is too complex (maximum {MAX_TOKENS} tokens)");
    }
    Ok(tokens)
}

fn normalize_expression(tokens: &[Token]) -> String {
    tokens.iter().map(Token::repr).collect::<Vec<_>>().join("")
}

#[derive(Debug, Clone)]
enum Expr {
    Number(f64),
    Unary {
        op: Operator,
        expr: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: Operator,
        right: Box<Expr>,
    },
}

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
    operations: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            position: 0,
            operations: 0,
        }
    }

    fn parse(mut self) -> Result<Expr> {
        let expr = self.parse_expression(0, 0)?;
        if self.position != self.tokens.len() {
            bail!(
                "unexpected token `{}` at position {}",
                self.tokens[self.position].repr(),
                self.position
            );
        }
        Ok(expr)
    }

    fn parse_expression(&mut self, min_prec: u8, depth: usize) -> Result<Expr> {
        if depth > MAX_AST_DEPTH {
            bail!("expression nesting is too deep (maximum {MAX_AST_DEPTH})");
        }

        let mut lhs = self.parse_prefix(depth)?;

        while let Some(op) = self.peek_binary_operator() {
            if op.precedence() < min_prec {
                break;
            }
            self.bump_operation_count()?;
            self.position += 1;
            let next_min = if op.is_right_associative() {
                op.precedence()
            } else {
                op.precedence().saturating_add(1)
            };
            let rhs = self.parse_expression(next_min, depth + 1)?;
            lhs = Expr::Binary {
                left: Box::new(lhs),
                op,
                right: Box::new(rhs),
            };
        }

        Ok(lhs)
    }

    fn parse_prefix(&mut self, depth: usize) -> Result<Expr> {
        let token = self
            .tokens
            .get(self.position)
            .ok_or_else(|| anyhow::anyhow!("unexpected end of expression"))?;

        match token {
            Token::Number { value, .. } => {
                self.position += 1;
                Ok(Expr::Number(*value))
            },
            Token::Minus => {
                self.bump_operation_count()?;
                self.position += 1;
                let expr = self.parse_expression(Operator::Power.precedence(), depth + 1)?;
                Ok(Expr::Unary {
                    op: Operator::Subtract,
                    expr: Box::new(expr),
                })
            },
            Token::Plus => {
                self.bump_operation_count()?;
                self.position += 1;
                let expr = self.parse_expression(Operator::Power.precedence(), depth + 1)?;
                Ok(Expr::Unary {
                    op: Operator::Add,
                    expr: Box::new(expr),
                })
            },
            Token::LParen => {
                self.position += 1;
                let expr = self.parse_expression(0, depth + 1)?;
                match self.tokens.get(self.position) {
                    Some(Token::RParen) => {
                        self.position += 1;
                        Ok(expr)
                    },
                    _ => bail!("missing closing ')' for parenthesized expression"),
                }
            },
            _ => bail!(
                "unexpected token `{}` at position {}",
                token.repr(),
                self.position
            ),
        }
    }

    fn peek_binary_operator(&self) -> Option<Operator> {
        match self.tokens.get(self.position) {
            Some(Token::Plus) => Some(Operator::Add),
            Some(Token::Minus) => Some(Operator::Subtract),
            Some(Token::Star) => Some(Operator::Multiply),
            Some(Token::Slash) => Some(Operator::Divide),
            Some(Token::Percent) => Some(Operator::Modulo),
            Some(Token::Caret) => Some(Operator::Power),
            _ => None,
        }
    }

    fn bump_operation_count(&mut self) -> Result<()> {
        self.operations = self.operations.saturating_add(1);
        if self.operations > MAX_OPERATIONS {
            bail!("expression is too complex (maximum {MAX_OPERATIONS} operations)");
        }
        Ok(())
    }
}

struct Evaluator<'a> {
    tokens: &'a [Token],
}

impl<'a> Evaluator<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens }
    }

    fn evaluate(&self) -> Result<f64> {
        let ast = Parser::new(self.tokens).parse()?;
        self.evaluate_expr(&ast)
    }

    fn evaluate_expr(&self, expr: &Expr) -> Result<f64> {
        match expr {
            Expr::Number(value) => Ok(*value),
            Expr::Unary { op, expr } => {
                let value = self.evaluate_expr(expr)?;
                match op {
                    Operator::Add => ensure_finite(value, "unary plus"),
                    Operator::Subtract => ensure_finite(-value, "unary minus"),
                    _ => bail!("unsupported unary operator `{}`", op.symbol()),
                }
            },
            Expr::Binary { left, op, right } => {
                let left = self.evaluate_expr(left)?;
                let right = self.evaluate_expr(right)?;
                match op {
                    Operator::Add => ensure_finite(left + right, "addition"),
                    Operator::Subtract => ensure_finite(left - right, "subtraction"),
                    Operator::Multiply => ensure_finite(left * right, "multiplication"),
                    Operator::Divide => {
                        if is_zero(right) {
                            bail!("division by zero");
                        }
                        ensure_finite(left / right, "division")
                    },
                    Operator::Modulo => {
                        if is_zero(right) {
                            bail!("modulo by zero");
                        }
                        ensure_finite(left % right, "modulo")
                    },
                    Operator::Power => {
                        if right.abs() > MAX_ABS_EXPONENT {
                            bail!(
                                "exponent out of allowed range (max absolute exponent: {MAX_ABS_EXPONENT})"
                            );
                        }
                        ensure_finite(left.powf(right), "power")
                    },
                }
            },
        }
    }
}

fn result_to_json(value: f64) -> Result<Value> {
    let normalized = normalize_negative_zero(value);
    if normalized.fract() == 0.0 && normalized >= i64::MIN as f64 && normalized <= i64::MAX as f64 {
        return Ok(json!(normalized as i64));
    }

    let number = serde_json::Number::from_f64(normalized)
        .ok_or_else(|| anyhow::anyhow!("result is not a finite JSON number"))?;
    Ok(Value::Number(number))
}

fn evaluate_expression(expression: &str) -> Result<(f64, String)> {
    if expression.len() > MAX_EXPRESSION_CHARS {
        bail!("expression is too long (maximum {MAX_EXPRESSION_CHARS} characters)");
    }

    let tokens = tokenize(expression)?;
    let normalized = normalize_expression(&tokens);
    let result = Evaluator::new(&tokens).evaluate()?;
    Ok((result, normalized))
}

#[cfg(target_arch = "wasm32")]
export!(CalcComponent);

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn compute(expr: &str) -> f64 {
        evaluate_expression(expr).unwrap().0
    }

    fn compute_err(expr: &str) -> String {
        evaluate_expression(expr).unwrap_err().to_string()
    }

    // --- Basic arithmetic ---

    #[test]
    fn addition() {
        assert_eq!(compute("2 + 3"), 5.0);
    }

    #[test]
    fn subtraction() {
        assert_eq!(compute("10 - 4"), 6.0);
    }

    #[test]
    fn multiplication() {
        assert_eq!(compute("6 * 7"), 42.0);
    }

    #[test]
    fn division() {
        assert_eq!(compute("15 / 3"), 5.0);
    }

    #[test]
    fn modulo() {
        assert_eq!(compute("17 % 5"), 2.0);
    }

    #[test]
    fn power() {
        assert_eq!(compute("2 ^ 8"), 256.0);
    }

    // --- Operator precedence ---

    #[test]
    fn mul_before_add() {
        assert_eq!(compute("2 + 3 * 4"), 14.0);
    }

    #[test]
    fn parentheses_override_precedence() {
        assert_eq!(compute("(2 + 3) * 4"), 20.0);
    }

    #[test]
    fn power_right_associative() {
        // 2^3^2 = 2^(3^2) = 2^9 = 512
        assert_eq!(compute("2 ^ 3 ^ 2"), 512.0);
    }

    #[test]
    fn mixed_precedence() {
        // 1 + 2 * 3 ^ 2 = 1 + 2*9 = 1 + 18 = 19
        assert_eq!(compute("1 + 2 * 3 ^ 2"), 19.0);
    }

    // --- Unary operators ---

    #[test]
    fn unary_minus() {
        assert_eq!(compute("-5"), -5.0);
    }

    #[test]
    fn unary_plus() {
        assert_eq!(compute("+3"), 3.0);
    }

    #[test]
    fn unary_minus_in_expression() {
        assert_eq!(compute("-(2 + 3)"), -5.0);
    }

    #[test]
    fn double_negative() {
        assert_eq!(compute("--5"), 5.0);
    }

    // --- Decimal and scientific notation ---

    #[test]
    fn decimal_numbers() {
        assert_eq!(compute("1.5 * 4"), 6.0);
    }

    #[test]
    fn leading_decimal() {
        assert_eq!(compute(".5 + .5"), 1.0);
    }

    #[test]
    fn scientific_notation() {
        assert_eq!(compute("1e3"), 1000.0);
    }

    #[test]
    fn scientific_negative_exponent() {
        assert_eq!(compute("2.5E-2"), 0.025);
    }

    #[test]
    fn scientific_positive_exponent() {
        assert_eq!(compute("1.5e+2"), 150.0);
    }

    // --- Error cases ---

    #[test]
    fn division_by_zero() {
        assert!(compute_err("1 / 0").contains("division by zero"));
    }

    #[test]
    fn modulo_by_zero() {
        assert!(compute_err("5 % 0").contains("modulo by zero"));
    }

    #[test]
    fn empty_expression() {
        assert!(compute_err("").contains("empty"));
    }

    #[test]
    fn whitespace_only() {
        assert!(compute_err("   ").contains("empty"));
    }

    #[test]
    fn unsupported_character() {
        assert!(compute_err("2 + x").contains("unsupported character"));
    }

    #[test]
    fn missing_closing_paren() {
        assert!(compute_err("(2 + 3").contains("missing closing ')'"));
    }

    #[test]
    fn unexpected_token() {
        assert!(compute_err("2 3").contains("unexpected token"));
    }

    #[test]
    fn expression_too_long() {
        let long = "1+".repeat(300);
        assert!(compute_err(&long).contains("too long"));
    }

    #[test]
    fn exponent_too_large() {
        assert!(compute_err("2 ^ 2000").contains("exponent out of allowed range"));
    }

    #[test]
    fn invalid_exponent_literal() {
        assert!(compute_err("1e").contains("invalid exponent"));
    }

    #[test]
    fn lone_dot_is_error() {
        assert!(compute_err(".").contains("invalid number literal"));
    }

    // --- Normalization ---

    #[test]
    fn normalized_expression_strips_spaces() {
        let (_, normalized) = evaluate_expression("2 + 3 * 4").unwrap();
        assert_eq!(normalized, "2+3*4");
    }

    #[test]
    fn negative_zero_normalised() {
        assert_eq!(normalize_negative_zero(-0.0).to_bits(), 0.0_f64.to_bits());
    }

    // --- result_to_json ---

    #[test]
    fn integer_result_serialized_as_integer() {
        let v = result_to_json(42.0).unwrap();
        assert_eq!(v, json!(42));
        assert!(v.is_i64());
    }

    #[test]
    fn float_result_serialized_as_float() {
        let v = result_to_json(1.5).unwrap();
        assert!(v.is_f64());
    }

    // --- execute_impl (JSON round-trip) ---

    #[test]
    fn execute_impl_basic() {
        let result = execute_impl(r#"{"expression": "2 + 2"}"#).unwrap();
        assert_eq!(result["result"], json!(4));
        assert_eq!(result["normalized_expr"], "2+2");
    }

    #[test]
    fn execute_impl_expr_alias() {
        let result = execute_impl(r#"{"expr": "3 * 3"}"#).unwrap();
        assert_eq!(result["result"], json!(9));
    }

    #[test]
    fn execute_impl_missing_param() {
        let err = execute_impl(r#"{"foo": "bar"}"#).unwrap_err();
        assert!(err.to_string().contains("missing 'expression' parameter"));
    }

    #[test]
    fn execute_impl_invalid_json() {
        assert!(execute_impl("not json").is_err());
    }
}
