use {
    crate::moltis::tool::types::{ToolError, ToolValue},
    anyhow::{Result, bail},
    serde_json::{Value, json},
};

wit_bindgen::generate!({
    path: "../../../wit",
    world: "pure-tool",
});

const MAX_EXPRESSION_CHARS: usize = 512;
const MAX_TOKENS: usize = 256;
const MAX_AST_DEPTH: usize = 64;
const MAX_OPERATIONS: usize = 512;
const MAX_ABS_EXPONENT: f64 = 1024.0;
const MAX_ABS_RESULT: f64 = 1.0e308;

struct CalcComponent;

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

export!(CalcComponent);
