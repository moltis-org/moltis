//! `calc` tool — evaluate strict arithmetic expressions without shell access.
//!
//! Supported operators: `+`, `-`, `*`, `/`, `%`, `^`, and parentheses.
//! This tool intentionally does not support variables, functions, or assignments.

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
};

use crate::{Result, error::Error};

const MAX_EXPRESSION_CHARS: usize = 512;
const MAX_TOKENS: usize = 256;
const MAX_AST_DEPTH: usize = 64;
const MAX_OPERATIONS: usize = 512;
const MAX_ABS_EXPONENT: f64 = 1024.0;
const MAX_ABS_RESULT: f64 = 1.0e308;

/// Arithmetic evaluator tool.
#[derive(Default)]
pub struct CalcTool;

impl CalcTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
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
        return Err(Error::message(format!(
            "{context} produced a non-finite result"
        )));
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
            return Err(Error::message(format!(
                "invalid number literal at byte {start}"
            )));
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
        return Err(Error::message(format!(
            "invalid number literal at byte {start}"
        )));
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
            return Err(Error::message(format!(
                "invalid exponent in number literal at byte {exponent_marker}"
            )));
        }
    }

    let repr = expression[start..i].to_string();
    let value = repr
        .parse::<f64>()
        .map_err(|_| Error::message(format!("invalid number literal `{repr}`")))?;
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
            _ => {
                let ch = expression[i..].chars().next().unwrap_or('\u{FFFD}');
                return Err(Error::message(format!(
                    "unsupported character `{ch}` in expression"
                )));
            },
        }

        if tokens.len() > MAX_TOKENS {
            return Err(Error::message(format!(
                "expression is too long (maximum {MAX_TOKENS} tokens)"
            )));
        }
    }

    if tokens.is_empty() {
        return Err(Error::message("expression is empty"));
    }

    Ok(tokens)
}

fn normalize_expression(tokens: &[Token]) -> String {
    let mut out = String::new();
    for token in tokens {
        out.push_str(&token.repr());
    }
    out
}

fn apply_binary(op: Operator, lhs: f64, rhs: f64) -> Result<f64> {
    let result = match op {
        Operator::Add => lhs + rhs,
        Operator::Subtract => lhs - rhs,
        Operator::Multiply => lhs * rhs,
        Operator::Divide => {
            if is_zero(rhs) {
                return Err(Error::message("division by zero is not allowed"));
            }
            lhs / rhs
        },
        Operator::Modulo => {
            if is_zero(rhs) {
                return Err(Error::message("modulo by zero is not allowed"));
            }
            lhs % rhs
        },
        Operator::Power => {
            if !rhs.is_finite() || rhs.abs() > MAX_ABS_EXPONENT {
                return Err(Error::message(format!(
                    "exponent out of allowed range (+/-{MAX_ABS_EXPONENT})"
                )));
            }
            lhs.powf(rhs)
        },
    };

    ensure_finite(result, op.symbol())
}

struct Evaluator<'a> {
    tokens: &'a [Token],
    pos: usize,
    operations: usize,
}

impl<'a> Evaluator<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            operations: 0,
        }
    }

    fn evaluate(mut self) -> Result<f64> {
        let value = self.parse_expression(0, 0)?;
        if let Some(token) = self.peek() {
            return Err(Error::message(format!(
                "unexpected token `{}`",
                token.repr()
            )));
        }
        ensure_finite(value, "expression")
    }

    fn parse_expression(&mut self, min_precedence: u8, depth: usize) -> Result<f64> {
        let mut lhs = self.parse_prefix(depth)?;

        while let Some(op) = self.peek_binary_operator() {
            let precedence = op.precedence();
            if precedence < min_precedence {
                break;
            }

            self.pos = self.pos.saturating_add(1);
            let rhs_precedence = if op.is_right_associative() {
                precedence
            } else {
                precedence.saturating_add(1)
            };
            let rhs = self.parse_expression(rhs_precedence, depth.saturating_add(1))?;
            lhs = apply_binary(op, lhs, rhs)?;
            self.bump_operation_count()?;
        }

        Ok(lhs)
    }

    fn parse_prefix(&mut self, depth: usize) -> Result<f64> {
        if depth > MAX_AST_DEPTH {
            return Err(Error::message(format!(
                "expression nesting is too deep (maximum {MAX_AST_DEPTH})"
            )));
        }

        match self.peek() {
            Some(Token::Number { value, .. }) => {
                let value = *value;
                self.pos = self.pos.saturating_add(1);
                Ok(value)
            },
            Some(Token::Plus) => {
                self.pos = self.pos.saturating_add(1);
                self.parse_prefix(depth.saturating_add(1))
            },
            Some(Token::Minus) => {
                self.pos = self.pos.saturating_add(1);
                let value = self.parse_prefix(depth.saturating_add(1))?;
                self.bump_operation_count()?;
                ensure_finite(-value, "unary -")
            },
            Some(Token::LParen) => {
                self.pos = self.pos.saturating_add(1);
                let value = self.parse_expression(0, depth.saturating_add(1))?;
                match self.peek() {
                    Some(Token::RParen) => {
                        self.pos = self.pos.saturating_add(1);
                        Ok(value)
                    },
                    _ => Err(Error::message("missing closing `)` ")),
                }
            },
            Some(other) => Err(Error::message(format!(
                "unexpected token `{}`",
                other.repr()
            ))),
            None => Err(Error::message("unexpected end of expression")),
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_binary_operator(&self) -> Option<Operator> {
        match self.peek() {
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
            return Err(Error::message(format!(
                "expression is too complex (maximum {MAX_OPERATIONS} operations)"
            )));
        }
        Ok(())
    }
}

fn result_to_json(value: f64) -> Result<Value> {
    let normalized = normalize_negative_zero(value);
    if normalized.fract() == 0.0 && normalized >= i64::MIN as f64 && normalized <= i64::MAX as f64 {
        return Ok(json!(normalized as i64));
    }

    let number = serde_json::Number::from_f64(normalized)
        .ok_or_else(|| Error::message("result is not a finite JSON number"))?;
    Ok(Value::Number(number))
}

fn evaluate_expression(expression: &str) -> Result<(f64, String)> {
    if expression.len() > MAX_EXPRESSION_CHARS {
        return Err(Error::message(format!(
            "expression is too long (maximum {MAX_EXPRESSION_CHARS} characters)"
        )));
    }

    let tokens = tokenize(expression)?;
    let normalized = normalize_expression(&tokens);
    let result = Evaluator::new(&tokens).evaluate()?;
    Ok((result, normalized))
}

#[async_trait]
impl AgentTool for CalcTool {
    fn name(&self) -> &str {
        "calc"
    }

    fn description(&self) -> &str {
        "Evaluate arithmetic expressions safely. Supports +, -, *, /, %, ^, unary +/- and parentheses. \
         No variables, functions, or assignments."
    }

    fn parameters_schema(&self) -> Value {
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
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let expression = params
            .get("expression")
            .or_else(|| params.get("expr"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'expression' parameter"))?;

        let (result, normalized_expr) = evaluate_expression(expression)?;
        Ok(json!({
            "result": result_to_json(result)?,
            "normalized_expr": normalized_expr
        }))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expression: &str) -> (f64, String) {
        evaluate_expression(expression).unwrap()
    }

    #[test]
    fn evaluates_operator_precedence() {
        let (value, normalized) = eval("2 + 3 * 4");
        assert_eq!(value, 14.0);
        assert_eq!(normalized, "2+3*4");
    }

    #[test]
    fn evaluates_parentheses_and_unary_minus() {
        let (value, normalized) = eval("-(2 + 3) * 4");
        assert_eq!(value, -20.0);
        assert_eq!(normalized, "-(2+3)*4");
    }

    #[test]
    fn power_is_right_associative() {
        let (value, _) = eval("2 ^ 3 ^ 2");
        assert_eq!(value, 512.0);
    }

    #[test]
    fn supports_floating_point_results() {
        let (value, _) = eval("1 / 2 + 3 % 2");
        assert_eq!(value, 1.5);
    }

    #[test]
    fn rejects_division_by_zero() {
        let err = evaluate_expression("10 / 0").unwrap_err().to_string();
        assert!(err.contains("division by zero"));
    }

    #[test]
    fn rejects_invalid_characters() {
        let err = evaluate_expression("2 + foo").unwrap_err().to_string();
        assert!(err.contains("unsupported character"));
    }

    #[test]
    fn rejects_too_large_exponent() {
        let err = evaluate_expression("2 ^ 4096").unwrap_err().to_string();
        assert!(err.contains("exponent out of allowed range"));
    }

    #[test]
    fn rejects_expressions_that_are_too_long() {
        let long_expr = "1".repeat(MAX_EXPRESSION_CHARS + 1);
        let err = evaluate_expression(&long_expr).unwrap_err().to_string();
        assert!(err.contains("expression is too long"));
    }

    #[tokio::test]
    async fn execute_returns_structured_result() {
        let tool = CalcTool::new();
        let value = tool
            .execute(json!({ "expression": " (10 + 2) / 3 " }))
            .await
            .unwrap();

        assert_eq!(value["normalized_expr"], "(10+2)/3");
        assert_eq!(value["result"], 4.0);
    }

    #[tokio::test]
    async fn execute_supports_expr_alias() {
        let tool = CalcTool::new();
        let value = tool.execute(json!({ "expr": "2^8" })).await.unwrap();
        assert_eq!(value["result"], 256);
    }
}
