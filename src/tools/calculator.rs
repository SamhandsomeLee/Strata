//! Calculator tool — evaluates basic arithmetic expressions (design §5, C14).

use crate::error::ToolError;
use crate::tool::{Tool, ToolSchema};

/// Stateless calculator: `+ - * /`, parentheses, integers and decimals.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Calculator;

impl Tool for Calculator {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "calculator".into(),
            description: "Evaluate a basic arithmetic expression (+, -, *, /, parentheses).".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "Arithmetic expression to evaluate, e.g. \"1+2*3\" or \"(10-4)/2\""
                    }
                },
                "required": ["expression"]
            }),
        }
    }

    fn execute(&self, args: serde_json::Value) -> Result<String, ToolError> {
        let expr = args
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing required field: expression".into()))?;

        let expr = expr.trim();
        if expr.is_empty() {
            return Err(ToolError::InvalidArgs("empty expression".into()));
        }

        validate_chars(expr)?;
        let value = evaluate(expr)?;
        Ok(format_number(value))
    }
}

/// Allowed: digits, `.`, operators, parentheses, whitespace.
fn validate_chars(expr: &str) -> Result<(), ToolError> {
    for ch in expr.chars() {
        if ch.is_ascii_digit()
            || matches!(ch, '+' | '-' | '*' | '/' | '(' | ')' | '.')
            || ch.is_whitespace()
        {
            continue;
        }
        return Err(ToolError::InvalidArgs(format!(
            "invalid character in expression: {ch:?}"
        )));
    }
    Ok(())
}

fn evaluate(expr: &str) -> Result<f64, ToolError> {
    let mut parser = Parser::new(expr);
    let value = parser.parse_expr()?;
    parser.expect_end()?;
    if !value.is_finite() {
        return Err(ToolError::ExecutionFailed(
            "result is not a finite number (overflow?)".into(),
        ));
    }
    Ok(value)
}

/// Largest magnitude where f64 still represents every integer exactly (2^53).
const MAX_EXACT_INT: f64 = 9_007_199_254_740_992.0;

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() <= MAX_EXACT_INT {
        return format!("{}", n.trunc() as i64);
    }
    format!("{n}")
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
        }
    }

    fn parse_expr(&mut self) -> Result<f64, ToolError> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            match self.peek_char() {
                Some('+') => {
                    self.next_char();
                    value += self.parse_term()?;
                }
                Some('-') => {
                    self.next_char();
                    value -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> Result<f64, ToolError> {
        let mut value = self.parse_factor()?;
        loop {
            self.skip_ws();
            match self.peek_char() {
                Some('*') => {
                    self.next_char();
                    value *= self.parse_factor()?;
                }
                Some('/') => {
                    self.next_char();
                    let rhs = self.parse_factor()?;
                    if rhs == 0.0 {
                        return Err(ToolError::ExecutionFailed("division by zero".into()));
                    }
                    value /= rhs;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_factor(&mut self) -> Result<f64, ToolError> {
        self.skip_ws();
        if self.peek_char() == Some('-') {
            self.next_char();
            return Ok(-self.parse_factor()?);
        }
        if self.peek_char() == Some('+') {
            self.next_char();
            return self.parse_factor();
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<f64, ToolError> {
        self.skip_ws();
        match self.peek_char() {
            Some('(') => {
                self.next_char();
                let value = self.parse_expr()?;
                self.skip_ws();
                if self.next_char() != Some(')') {
                    return Err(ToolError::InvalidArgs(
                        "missing closing parenthesis".into(),
                    ));
                }
                Ok(value)
            }
            Some(ch) if ch.is_ascii_digit() || ch == '.' => self.parse_number(),
            _ => Err(ToolError::InvalidArgs(
                "expected number or '('".into(),
            )),
        }
    }

    fn parse_number(&mut self) -> Result<f64, ToolError> {
        self.skip_ws();
        let mut consumed = String::new();
        let mut has_dot = false;
        let mut has_digit = false;

        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_digit() {
                has_digit = true;
            } else if ch == '.' && !has_dot {
                has_dot = true;
            } else {
                break;
            }
            consumed.push(ch);
            self.next_char();
        }

        if !has_digit {
            return Err(ToolError::InvalidArgs("invalid number".into()));
        }

        consumed
            .parse::<f64>()
            .map_err(|_| ToolError::InvalidArgs(format!("invalid number: {consumed}")))
    }

    fn expect_end(&mut self) -> Result<(), ToolError> {
        self.skip_ws();
        if self.peek_char().is_some() {
            return Err(ToolError::InvalidArgs(
                "unexpected trailing characters".into(),
            ));
        }
        Ok(())
    }

    fn skip_ws(&mut self) {
        while self.peek_char().is_some_and(|c| c.is_whitespace()) {
            self.next_char();
        }
    }

    fn peek_char(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn next_char(&mut self) -> Option<char> {
        self.chars.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_calculator_name() {
        let schema = Calculator.schema();
        assert_eq!(schema.name, "calculator");
        assert_eq!(schema.parameters["required"], serde_json::json!(["expression"]));
    }

    #[test]
    fn execute_simple_addition() {
        let result = Calculator
            .execute(serde_json::json!({ "expression": "1+2" }))
            .expect("ok");
        assert_eq!(result, "3");
    }

    #[test]
    fn execute_precedence() {
        let result = Calculator
            .execute(serde_json::json!({ "expression": "1+2*3" }))
            .expect("ok");
        assert_eq!(result, "7");
    }

    #[test]
    fn execute_parentheses() {
        let result = Calculator
            .execute(serde_json::json!({ "expression": "(1+2)*3" }))
            .expect("ok");
        assert_eq!(result, "9");
    }

    #[test]
    fn execute_float() {
        let result = Calculator
            .execute(serde_json::json!({ "expression": "10/4" }))
            .expect("ok");
        assert_eq!(result, "2.5");
    }

    #[test]
    fn execute_unary_minus() {
        let result = Calculator
            .execute(serde_json::json!({ "expression": "-3+5" }))
            .expect("ok");
        assert_eq!(result, "2");
    }

    #[test]
    fn missing_expression() {
        let err = Calculator.execute(serde_json::json!({})).unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn invalid_characters() {
        let err = Calculator
            .execute(serde_json::json!({ "expression": "1;drop" }))
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn divide_by_zero() {
        let err = Calculator
            .execute(serde_json::json!({ "expression": "1/0" }))
            .unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
    }

    #[test]
    fn divide_by_tiny_but_nonzero() {
        let result = Calculator
            .execute(serde_json::json!({ "expression": "1/0.0000000000000001" }))
            .expect("ok");
        assert_eq!(result, "10000000000000000");
    }

    #[test]
    fn large_result_uses_float_formatting() {
        // Beyond 2^53 integers lose exactness; must not saturate via `as i64`.
        let result = Calculator
            .execute(serde_json::json!({ "expression": "99999999999999999999+1" }))
            .expect("ok");
        assert_eq!(result, "100000000000000000000");
    }

    #[test]
    fn overflow_is_execution_error() {
        let huge = "9".repeat(309);
        let err = Calculator
            .execute(serde_json::json!({ "expression": huge }))
            .unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
    }

    #[test]
    fn register_in_registry() {
        let mut registry = crate::ToolRegistry::new();
        registry.register(Box::new(Calculator));
        assert!(registry.get("calculator").is_some());
        let out = registry
            .get("calculator")
            .unwrap()
            .execute(serde_json::json!({ "expression": "2*3" }))
            .expect("execute");
        assert_eq!(out, "6");
    }
}
