//! CLEM syntax tree and parser.
//!
//! Precedence follows IBM's Table 17 (lower binds tighter): `div rem mod` (2), `**` (3),
//! `* /` (4), `+ - ><` (5), `> >= < <=` (6), `= == /= /==` (7), `and` (9), `or` (10). Binary
//! operators associate to the left; unary minus binds tightest. The user guide contradicts
//! itself on `**` versus `div`/`rem` and is silent on associativity and unary minus; these are
//! listed for oracle probes in docs/clem/reference.md section 10.

use crate::ClemError;
use crate::lexer::{Token, tokenize};
use dms_types::Value;

/// Nesting bound (parentheses, unary operators, arguments): deeper expressions are rejected
/// instead of exhausting the stack.
const MAX_DEPTH: usize = 64;
/// Binary operators per expression: bounds the work for hostile input (chains themselves are
/// flat and do not deepen the tree).
const MAX_OPERATORS: usize = 1 << 16;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    /// A bare name that is not a function: a field.
    Field(String),
    /// `'...'`: a field if the data model has it, otherwise a string.
    Quoted(String),
    /// `@NAME` or another zero-argument special (`@FIELD`, `@PARTITION_FIELD`, `@INDEX`, ...).
    Special(String),
    Negate(Box<Expr>),
    /// One precedence level applied left to right: `first op1 e1 op2 e2 ...`. Kept flat so long
    /// operator chains do not deepen the tree.
    Chain(Box<Expr>, Vec<(BinaryOp, Expr)>),
    If {
        branches: Vec<(Expr, Expr)>,
        otherwise: Box<Expr>,
    },
    Call(String, Vec<Expr>),
    List(Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    Div,
    Rem,
    Concat,
    Equal,
    NotEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    And,
    Or,
}

pub fn parse(source: &str) -> Result<Expr, ClemError> {
    let tokens = tokenize(source)?;
    let operators = tokens
        .iter()
        .filter(|(_, token)| match token {
            Token::Op(_) => true,
            Token::Name(name) => matches!(name.as_str(), "and" | "or" | "div" | "rem" | "mod"),
            _ => false,
        })
        .count();
    if operators > MAX_OPERATORS {
        return Err(ClemError::Syntax {
            position: 0,
            message: format!("more than {MAX_OPERATORS} operators"),
        });
    }
    let mut parser = Parser {
        tokens,
        next: 0,
        depth: 0,
        end: source.len(),
    };
    let expr = parser.expression()?;
    match parser.peek() {
        None => Ok(expr),
        Some(_) => Err(parser.error("unexpected token after the expression")),
    }
}

struct Parser {
    tokens: Vec<(usize, Token)>,
    next: usize,
    depth: usize,
    end: usize,
}

/// One binary precedence level: the operators it accepts and how to read the next level.
type Level = fn(&Token) -> Option<BinaryOp>;

const LEVELS: [Level; 7] = [
    |t| matches!(t, Token::Name(n) if n == "or").then_some(BinaryOp::Or),
    |t| matches!(t, Token::Name(n) if n == "and").then_some(BinaryOp::And),
    |t| match t {
        Token::Op("=" | "==") => Some(BinaryOp::Equal),
        Token::Op("/=" | "/==") => Some(BinaryOp::NotEqual),
        _ => None,
    },
    |t| match t {
        Token::Op(">") => Some(BinaryOp::Greater),
        Token::Op(">=") => Some(BinaryOp::GreaterEqual),
        Token::Op("<") => Some(BinaryOp::Less),
        Token::Op("<=") => Some(BinaryOp::LessEqual),
        _ => None,
    },
    |t| match t {
        Token::Op("+") => Some(BinaryOp::Add),
        Token::Op("-") => Some(BinaryOp::Subtract),
        Token::Op("><") => Some(BinaryOp::Concat),
        _ => None,
    },
    |t| match t {
        Token::Op("*") => Some(BinaryOp::Multiply),
        Token::Op("/") => Some(BinaryOp::Divide),
        _ => None,
    },
    |t| matches!(t, Token::Op("**")).then_some(BinaryOp::Power),
];

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.next).map(|(_, token)| token)
    }

    fn position(&self) -> usize {
        self.tokens.get(self.next).map_or(self.end, |(p, _)| *p)
    }

    fn error(&self, message: &str) -> ClemError {
        ClemError::Syntax {
            position: self.position(),
            message: message.to_string(),
        }
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.next).map(|(_, t)| t.clone());
        self.next += 1;
        token
    }

    fn is_name(&self, name: &str) -> bool {
        matches!(self.peek(), Some(Token::Name(n)) if n == name)
    }

    fn expect_name(&mut self, name: &str) -> Result<(), ClemError> {
        if self.is_name(name) {
            self.next += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected `{name}`")))
        }
    }

    fn expect(&mut self, wanted: &Token, what: &str) -> Result<(), ClemError> {
        if self.peek() == Some(wanted) {
            self.next += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected {what}")))
        }
    }

    fn expression(&mut self) -> Result<Expr, ClemError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("expression nested too deeply"));
        }
        let result = self.level(0);
        self.depth -= 1;
        result
    }

    fn level(&mut self, index: usize) -> Result<Expr, ClemError> {
        let Some(level) = LEVELS.get(index) else {
            return self.div_rem();
        };
        let first = self.level(index + 1)?;
        let mut rest = Vec::new();
        while let Some(op) = self.peek().and_then(level) {
            self.next += 1;
            rest.push((op, self.level(index + 1)?));
        }
        Ok(chain(first, rest))
    }

    fn div_rem(&mut self) -> Result<Expr, ClemError> {
        let first = self.unary()?;
        let mut rest = Vec::new();
        loop {
            let op = match self.peek() {
                Some(Token::Name(n)) if n == "div" => BinaryOp::Div,
                Some(Token::Name(n)) if n == "rem" || n == "mod" => BinaryOp::Rem,
                _ => return Ok(chain(first, rest)),
            };
            self.next += 1;
            rest.push((op, self.unary()?));
        }
    }

    fn unary(&mut self) -> Result<Expr, ClemError> {
        match self.peek() {
            Some(Token::Op("-")) => {
                self.next += 1;
                self.nested(|parser| parser.unary()).map(|expr| match expr {
                    Expr::Literal(Value::Integer(i)) => match i.checked_neg() {
                        Some(negated) => Expr::Literal(Value::Integer(negated)),
                        None => Expr::Negate(Box::new(Expr::Literal(Value::Integer(i)))),
                    },
                    Expr::Literal(Value::Real(r)) => Expr::Literal(Value::Real(-r)),
                    other => Expr::Negate(Box::new(other)),
                })
            }
            Some(Token::Op("+")) => {
                self.next += 1;
                self.nested(|parser| parser.unary())
            }
            _ => self.primary(),
        }
    }

    fn nested(
        &mut self,
        parse: impl FnOnce(&mut Parser) -> Result<Expr, ClemError>,
    ) -> Result<Expr, ClemError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("expression nested too deeply"));
        }
        let result = parse(self);
        self.depth -= 1;
        result
    }

    fn primary(&mut self) -> Result<Expr, ClemError> {
        let position = self.position();
        let Some(token) = self.advance() else {
            return Err(ClemError::Syntax {
                position,
                message: "expression ends too early".into(),
            });
        };
        match token {
            Token::Integer(i) => Ok(Expr::Literal(Value::Integer(i))),
            Token::Real(r) => Ok(Expr::Literal(Value::real(r))),
            Token::String(s) => Ok(Expr::Literal(Value::string(s))),
            Token::Char(c) => Ok(Expr::Literal(Value::string(c.to_string()))),
            Token::Quoted(name) => Ok(Expr::Quoted(name)),
            Token::LParen => {
                let inner = self.expression()?;
                self.expect(&Token::RParen, "`)`")?;
                Ok(inner)
            }
            Token::LBracket => self.list(),
            Token::Name(name) if name == "if" => self.conditional(),
            Token::Name(name) if name == "undef" => Ok(Expr::Literal(Value::Null)),
            Token::Name(name) => {
                if self.peek() == Some(&Token::LParen) {
                    self.next += 1;
                    let args = self.arguments()?;
                    Ok(Expr::Call(name, args))
                } else if name.starts_with('@') {
                    Ok(Expr::Special(name))
                } else {
                    Ok(Expr::Field(name))
                }
            }
            Token::RParen | Token::RBracket | Token::Comma | Token::Op(_) => {
                Err(ClemError::Syntax {
                    position,
                    message: "expected a value".into(),
                })
            }
        }
    }

    fn arguments(&mut self) -> Result<Vec<Expr>, ClemError> {
        let mut args = Vec::new();
        if self.peek() == Some(&Token::RParen) {
            self.next += 1;
            return Ok(args);
        }
        loop {
            args.push(self.expression()?);
            match self.advance() {
                Some(Token::Comma) => {}
                Some(Token::RParen) => return Ok(args),
                _ => return Err(self.error("expected `,` or `)`")),
            }
        }
    }

    /// `[a b c]` or `[a, b, c]` (both appear in IBM's material).
    fn list(&mut self) -> Result<Expr, ClemError> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Some(Token::RBracket) => {
                    self.next += 1;
                    return Ok(Expr::List(items));
                }
                Some(Token::Comma) => self.next += 1,
                Some(_) => items.push(self.expression()?),
                None => return Err(self.error("unterminated list")),
            }
        }
    }

    /// `if C then E {elseif C then E} else E endif`.
    fn conditional(&mut self) -> Result<Expr, ClemError> {
        let mut branches = Vec::new();
        let condition = self.expression()?;
        self.expect_name("then")?;
        branches.push((condition, self.expression()?));
        loop {
            if self.is_name("elseif") {
                self.next += 1;
                let condition = self.expression()?;
                self.expect_name("then")?;
                branches.push((condition, self.expression()?));
            } else if self.is_name("else") {
                self.next += 1;
                let otherwise = self.expression()?;
                self.expect_name("endif")?;
                return Ok(Expr::If {
                    branches,
                    otherwise: Box::new(otherwise),
                });
            } else {
                return Err(self.error("expected `elseif` or `else`"));
            }
        }
    }
}

fn chain(first: Expr, rest: Vec<(BinaryOp, Expr)>) -> Expr {
    if rest.is_empty() {
        first
    } else {
        Expr::Chain(Box::new(first), rest)
    }
}
