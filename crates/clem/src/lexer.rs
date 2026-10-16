//! Tokens of CLEM (docs/clem/reference.md section 1).

use crate::ClemError;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Integer(i64),
    Real(f64),
    /// `"..."`: always a string.
    String(String),
    /// `'...'`: a field when the data model has it, otherwise a string.
    Quoted(String),
    /// `` `c` ``
    Char(char),
    /// Names, keywords and `@` names.
    Name(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Op(&'static str),
}

/// Operators, longest first so that `/==` wins over `/=` and `/`.
const OPERATORS: &[&str] = &[
    "&&/=_0", "&&=_0", "&&~~", "||/&", "/==", "**", "/=", "==", "><", ">=", "<=", "&&", "||", "<<",
    ">>", "~~", "*", "/", "+", "-", ">", "<", "=",
];

/// Maximum expression length in characters.
const MAX_LENGTH: usize = 1 << 20;

pub fn tokenize(source: &str) -> Result<Vec<(usize, Token)>, ClemError> {
    if source.len() > MAX_LENGTH {
        return Err(ClemError::Syntax {
            position: 0,
            message: "expression too long".into(),
        });
    }
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(&(position, c)) = chars.get(i) {
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let syntax = |message: &str| ClemError::Syntax {
            position,
            message: message.to_string(),
        };
        match c {
            // Comments (oracle: `#` to the end of the line and `/* ... */`; `//` is no comment).
            '#' => {
                while chars.get(i).is_some_and(|&(_, c)| c != '\n') {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1).is_some_and(|&(_, c)| c == '*') => {
                i += 2;
                loop {
                    match (chars.get(i), chars.get(i + 1)) {
                        (Some(&(_, '*')), Some(&(_, '/'))) => break,
                        (Some(_), _) => i += 1,
                        (None, _) => return Err(syntax("unterminated comment")),
                    }
                }
                i += 2;
            }
            '(' => push(&mut tokens, &mut i, position, Token::LParen),
            ')' => push(&mut tokens, &mut i, position, Token::RParen),
            '[' => push(&mut tokens, &mut i, position, Token::LBracket),
            ']' => push(&mut tokens, &mut i, position, Token::RBracket),
            ',' => push(&mut tokens, &mut i, position, Token::Comma),
            '"' => {
                let (text, next) =
                    delimited(&chars, i + 1, '"').ok_or_else(|| syntax("unterminated string"))?;
                tokens.push((position, Token::String(text)));
                i = next;
            }
            '\'' => {
                let (text, next) = delimited(&chars, i + 1, '\'')
                    .ok_or_else(|| syntax("unterminated quoted name"))?;
                tokens.push((position, Token::Quoted(text)));
                i = next;
            }
            '`' => match (chars.get(i + 1), chars.get(i + 2)) {
                (Some(&(_, ch)), Some(&(_, '`'))) => {
                    tokens.push((position, Token::Char(ch)));
                    i += 3;
                }
                _ => return Err(syntax("unterminated character")),
            },
            '0'..='9' => {
                let (token, next) =
                    number(source, &chars, i).ok_or_else(|| syntax("bad number"))?;
                tokens.push((position, token));
                i = next;
            }
            c if c.is_alphabetic() || c == '_' || c == '@' || c == '$' => {
                let start = i;
                i += 1;
                while chars
                    .get(i)
                    .is_some_and(|&(_, c)| c.is_alphanumeric() || c == '_')
                {
                    i += 1;
                }
                tokens.push((position, Token::Name(slice(source, &chars, start, i))));
            }
            _ => {
                let rest = source.get(position..).unwrap_or_default();
                let op = OPERATORS
                    .iter()
                    .find(|op| rest.starts_with(**op))
                    .ok_or_else(|| syntax("unexpected character"))?;
                tokens.push((position, Token::Op(op)));
                i += op.chars().count();
            }
        }
    }
    Ok(tokens)
}

fn push(tokens: &mut Vec<(usize, Token)>, i: &mut usize, position: usize, token: Token) {
    tokens.push((position, token));
    *i += 1;
}

fn slice(source: &str, chars: &[(usize, char)], start: usize, end: usize) -> String {
    let from = chars.get(start).map_or(source.len(), |&(p, _)| p);
    let to = chars.get(end).map_or(source.len(), |&(p, _)| p);
    source.get(from..to).unwrap_or_default().to_string()
}

/// Text up to the closing delimiter. Both quote kinds take the escapes the oracle confirmed
/// (docs/clem/oracle-results.md rule 41): `\t`, `\n`, `\\` and the escaped delimiter; any other
/// backslash stays as written (`"A"` has six characters). Doubled quotes are no escape.
fn delimited(chars: &[(usize, char)], mut i: usize, end: char) -> Option<(String, usize)> {
    let mut text = String::new();
    loop {
        let &(_, c) = chars.get(i)?;
        if c == end {
            return Some((text, i + 1));
        }
        if c == '\\' {
            let &(_, escaped) = chars.get(i + 1)?;
            match escaped {
                't' => text.push('\t'),
                'n' => text.push('\n'),
                '\\' => text.push('\\'),
                e if e == end => text.push(e),
                other => {
                    text.push('\\');
                    text.push(other);
                }
            }
            i += 2;
        } else {
            text.push(c);
            i += 1;
        }
    }
}

/// `digits [. digits] [e [+-] digits]`; integers that overflow `i64` become reals.
fn number(source: &str, chars: &[(usize, char)], start: usize) -> Option<(Token, usize)> {
    let digit = |i: usize| chars.get(i).is_some_and(|&(_, c)| c.is_ascii_digit());
    let mut i = start;
    while digit(i) {
        i += 1;
    }
    let mut real = false;
    if chars.get(i).is_some_and(|&(_, c)| c == '.') && digit(i + 1) {
        real = true;
        i += 1;
        while digit(i) {
            i += 1;
        }
    }
    if chars.get(i).is_some_and(|&(_, c)| c == 'e' || c == 'E') {
        let mut j = i + 1;
        if chars.get(j).is_some_and(|&(_, c)| c == '+' || c == '-') {
            j += 1;
        }
        if digit(j) {
            real = true;
            i = j;
            while digit(i) {
                i += 1;
            }
        }
    }
    let text = slice(source, chars, start, i);
    let token = if real {
        Token::Real(text.parse().ok()?)
    } else {
        match text.parse::<i64>() {
            Ok(value) => Token::Integer(value),
            Err(_) => Token::Real(text.parse().ok()?),
        }
    };
    Some((token, i))
}
