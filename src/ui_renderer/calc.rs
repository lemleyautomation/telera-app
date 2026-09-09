//! Compile-time arithmetic for TML layout files.
//!
//! A tiny expression language evaluated *once*, while a layout file is parsed,
//! and folded to a constant `f32`. It never runs at frame time - the result
//! lands in a `` `set-numeric` `` declaration as a plain
//! [`DataSrc::Static`](super::layout_runner::DataSrc) number, indistinguishable
//! from a literal.
//!
//! Two entry points, both used only by [`super::layout_runner`]:
//! - [`parse_fn`] reads a `` - `calc` `name(a, b) = a * b` `` header directive
//!   into a named [`CalcFn`].
//! - [`parse_expr`] + [`eval`] turn a `` `set-numeric` `` value like
//!   `` `scale(width, 2) + 4` `` into a number, resolving identifiers against a
//!   stack of in-scope static declarations and calls against the file's
//!   `` `calc` `` functions.
//!
//! Grammar (all left-associative; `%` is float remainder):
//! ```text
//! expr    := term (('+' | '-') term)*
//! term    := unary (('*' | '/' | '%') unary)*
//! unary   := '-' unary | primary
//! primary := number | ident | ident '(' (expr (',' expr)*)? ')' | '(' expr ')'
//! ```
//! There are no built-in functions: `min`, `clamp`, ... are whatever the file
//! defines with `` `calc` ``.

use std::collections::HashMap;
use std::fmt;

/// The recursion budget for nested `` `calc` `` function calls. A cyclic
/// definition (`` `r(n) = r(n)` ``) trips this instead of overflowing the
/// stack.
const MAX_DEPTH: u32 = 32;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    Num(f32),
    Ident(Box<str>),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(Box<str>, Vec<Expr>),
}

/// A user-defined `` `calc` `` function: its parameter names and its body. The
/// body's free identifiers must all be parameters (checked lazily, at [`eval`]
/// time, when the surrounding environment is known).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CalcFn {
    pub params: Vec<Box<str>>,
    pub body: Expr,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CalcError {
    /// Lexing or parsing failed (bad character, unbalanced parens, trailing
    /// input, a `` `calc` `` directive with no `=`, ...).
    Syntax(String),
    /// An identifier that is not an in-scope static declaration (or, inside a
    /// function body, not a parameter). Referencing a `get-*` binding or an app
    /// field lands here - those are runtime-only.
    UnknownIdent(String),
    /// A call to a name that is not a defined `` `calc` `` function.
    UnknownFn(String),
    /// A `` `calc` `` function called with the wrong number of arguments.
    Arity {
        name: String,
        expected: usize,
        got: usize,
    },
    /// Division or remainder by zero (rejected rather than folded to `inf`/`NaN`).
    DivByZero,
    /// Nested calls exceeded [`MAX_DEPTH`] - almost always a cyclic definition.
    Recursion,
}

impl fmt::Display for CalcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CalcError::Syntax(msg) => write!(f, "syntax error: {msg}"),
            CalcError::UnknownIdent(name) => write!(
                f,
                "unknown value `{name}` (not a static declaration in scope; \
                 `get-*` bindings and app fields cannot be used in a calc expression)"
            ),
            CalcError::UnknownFn(name) => write!(f, "unknown function `{name}`"),
            CalcError::Arity {
                name,
                expected,
                got,
            } => write!(
                f,
                "function `{name}` takes {expected} argument(s), got {got}"
            ),
            CalcError::DivByZero => write!(f, "division by zero"),
            CalcError::Recursion => write!(f, "calc function recursion limit reached"),
        }
    }
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f32),
    Ident(Box<str>),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    LParen,
    RParen,
    Comma,
    Eq,
}

fn lex(src: &str) -> Result<Vec<Tok>, CalcError> {
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            b'-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            b'*' => {
                out.push(Tok::Star);
                i += 1;
            }
            b'/' => {
                out.push(Tok::Slash);
                i += 1;
            }
            b'%' => {
                out.push(Tok::Percent);
                i += 1;
            }
            b'(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            b')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            b',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            b'=' => {
                out.push(Tok::Eq);
                i += 1;
            }
            b'0'..=b'9' | b'.' => {
                let start = i;
                let mut seen_dot = false;
                while i < bytes.len() {
                    match bytes[i] {
                        b'0'..=b'9' => i += 1,
                        b'.' if !seen_dot => {
                            seen_dot = true;
                            i += 1;
                        }
                        _ => break,
                    }
                }
                let text = &src[start..i];
                let value = text
                    .parse::<f32>()
                    .map_err(|_| CalcError::Syntax(format!("bad number `{text}`")))?;
                out.push(Tok::Num(value));
            }
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let start = i;
                // `-` is always subtraction here, never part of a name, so an
                // identifier in an expression can only use letters, digits and
                // `_` - a declaration whose name has other characters can't be
                // referenced from a `calc` expression.
                while i < bytes.len()
                    && matches!(bytes[i], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')
                {
                    i += 1;
                }
                // Interned verbatim - must match the `set-numeric` declaration
                // (or `calc` function) name exactly, like every other TML name.
                out.push(Tok::Ident(src[start..i].into()));
            }
            other => {
                return Err(CalcError::Syntax(format!(
                    "unexpected character `{}`",
                    other as char
                )));
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Parser (precedence climbing)
// ---------------------------------------------------------------------------

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(toks: &'a [Tok]) -> Self {
        Parser { toks, pos: 0 }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<&Tok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, want: &Tok) -> Result<(), CalcError> {
        match self.peek() {
            Some(t) if t == want => {
                self.pos += 1;
                Ok(())
            }
            Some(t) => Err(CalcError::Syntax(format!("expected `{want:?}`, found `{t:?}`"))),
            None => Err(CalcError::Syntax(format!("expected `{want:?}`, found end of input"))),
        }
    }

    /// `expr := term (('+' | '-') term)*`
    fn expr(&mut self) -> Result<Expr, CalcError> {
        let mut lhs = self.term()?;
        while let Some(op) = match self.peek() {
            Some(Tok::Plus) => Some(BinOp::Add),
            Some(Tok::Minus) => Some(BinOp::Sub),
            _ => None,
        } {
            self.pos += 1;
            let rhs = self.term()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `term := unary (('*' | '/' | '%') unary)*`
    fn term(&mut self) -> Result<Expr, CalcError> {
        let mut lhs = self.unary()?;
        while let Some(op) = match self.peek() {
            Some(Tok::Star) => Some(BinOp::Mul),
            Some(Tok::Slash) => Some(BinOp::Div),
            Some(Tok::Percent) => Some(BinOp::Rem),
            _ => None,
        } {
            self.pos += 1;
            let rhs = self.unary()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `unary := '-' unary | primary`
    fn unary(&mut self) -> Result<Expr, CalcError> {
        if let Some(Tok::Minus) = self.peek() {
            self.pos += 1;
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    /// `primary := number | ident | ident '(' args? ')' | '(' expr ')'`
    fn primary(&mut self) -> Result<Expr, CalcError> {
        match self.bump().cloned() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Ident(name)) => {
                if let Some(Tok::LParen) = self.peek() {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Tok::RParen)) {
                        loop {
                            args.push(self.expr()?);
                            match self.peek() {
                                Some(Tok::Comma) => {
                                    self.pos += 1;
                                }
                                _ => break,
                            }
                        }
                    }
                    self.eat(&Tok::RParen)?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            Some(Tok::LParen) => {
                let inner = self.expr()?;
                self.eat(&Tok::RParen)?;
                Ok(inner)
            }
            Some(other) => Err(CalcError::Syntax(format!("unexpected `{other:?}`"))),
            None => Err(CalcError::Syntax("unexpected end of input".to_string())),
        }
    }
}

/// Parses a complete expression, rejecting any trailing tokens.
pub(crate) fn parse_expr(src: &str) -> Result<Expr, CalcError> {
    let toks = lex(src)?;
    let mut parser = Parser::new(&toks);
    let expr = parser.expr()?;
    if parser.pos != toks.len() {
        return Err(CalcError::Syntax(format!(
            "unexpected `{:?}` after expression",
            toks[parser.pos]
        )));
    }
    Ok(expr)
}

/// Parses a `` `calc` `` header directive body: `name(p1, p2) = <expr>`.
/// Returns the (verbatim) function name and its [`CalcFn`].
pub(crate) fn parse_fn(signature_and_body: &str) -> Result<(String, CalcFn), CalcError> {
    let (signature, body_src) = signature_and_body
        .split_once('=')
        .ok_or_else(|| CalcError::Syntax("a `calc` function needs `name(args) = expression`".into()))?;

    let signature = signature.trim();
    let open = signature
        .find('(')
        .ok_or_else(|| CalcError::Syntax("a `calc` function needs a `(` parameter list".into()))?;
    let close = signature
        .rfind(')')
        .ok_or_else(|| CalcError::Syntax("a `calc` function needs a closing `)`".into()))?;
    if close < open {
        return Err(CalcError::Syntax("mismatched `(` / `)` in the parameter list".into()));
    }

    let name = signature[..open].trim();
    if name.is_empty() || !is_ident(name) {
        return Err(CalcError::Syntax(format!("`{name}` is not a valid function name")));
    }

    let mut params = Vec::new();
    let inside = signature[open + 1..close].trim();
    if !inside.is_empty() {
        for part in inside.split(',') {
            let param = part.trim();
            if !is_ident(param) {
                return Err(CalcError::Syntax(format!("`{param}` is not a valid parameter name")));
            }
            params.push(param.into());
        }
    }

    let body = parse_expr(body_src.trim())?;
    Ok((name.to_string(), CalcFn { params, body }))
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Evaluates `expr` against a stack of variable frames (`vars`, innermost last)
/// and the file's `` `calc` `` functions. `depth` guards against cyclic
/// function definitions - callers start at `0`.
pub(crate) fn eval(
    expr: &Expr,
    vars: &[HashMap<String, f32>],
    fns: &HashMap<String, CalcFn>,
    depth: u32,
) -> Result<f32, CalcError> {
    match expr {
        Expr::Num(n) => Ok(*n),
        Expr::Ident(name) => vars
            .iter()
            .rev()
            .find_map(|frame| frame.get(name.as_ref()).copied())
            .ok_or_else(|| CalcError::UnknownIdent(name.to_string())),
        Expr::Neg(inner) => Ok(-eval(inner, vars, fns, depth)?),
        Expr::Bin(op, lhs, rhs) => {
            let a = eval(lhs, vars, fns, depth)?;
            let b = eval(rhs, vars, fns, depth)?;
            Ok(match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => {
                    if b == 0.0 {
                        return Err(CalcError::DivByZero);
                    }
                    a / b
                }
                BinOp::Rem => {
                    if b == 0.0 {
                        return Err(CalcError::DivByZero);
                    }
                    a % b
                }
            })
        }
        Expr::Call(name, args) => {
            if depth >= MAX_DEPTH {
                return Err(CalcError::Recursion);
            }
            let func = fns
                .get(name.as_ref())
                .ok_or_else(|| CalcError::UnknownFn(name.to_string()))?;
            if func.params.len() != args.len() {
                return Err(CalcError::Arity {
                    name: name.to_string(),
                    expected: func.params.len(),
                    got: args.len(),
                });
            }
            let mut frame = HashMap::with_capacity(args.len());
            for (param, arg) in func.params.iter().zip(args) {
                let value = eval(arg, vars, fns, depth)?;
                frame.insert(param.to_string(), value);
            }
            // A function body sees only its own parameters, never the caller's
            // scope - `calc` functions are pure.
            eval(&func.body, std::slice::from_ref(&frame), fns, depth + 1)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_str(src: &str, vars: &[HashMap<String, f32>], fns: &HashMap<String, CalcFn>) -> Result<f32, CalcError> {
        eval(&parse_expr(src)?, vars, fns, 0)
    }

    fn no_vars() -> Vec<HashMap<String, f32>> {
        vec![HashMap::new()]
    }

    fn no_fns() -> HashMap<String, CalcFn> {
        HashMap::new()
    }

    #[test]
    fn arithmetic_and_precedence() {
        let v = no_vars();
        let f = no_fns();
        assert_eq!(eval_str("2 + 3 * 4", &v, &f).unwrap(), 14.0);
        assert_eq!(eval_str("(2 + 3) * 4", &v, &f).unwrap(), 20.0);
        assert_eq!(eval_str("2 * 45 / 34 * (56 + 4)", &v, &f).unwrap(), 2.0 * 45.0 / 34.0 * 60.0);
        assert_eq!(eval_str("10 % 3", &v, &f).unwrap(), 1.0);
        assert_eq!(eval_str("2 * -3", &v, &f).unwrap(), -6.0);
        assert_eq!(eval_str("-2 * 3", &v, &f).unwrap(), -6.0);
        assert_eq!(eval_str("1.5 + .5", &v, &f).unwrap(), 2.0);
    }

    #[test]
    fn hyphen_is_always_subtraction() {
        let f = no_fns();
        let mut frame = HashMap::new();
        frame.insert("base".to_string(), 10.0);
        let vars = vec![frame];
        // no spaces around `-`
        assert_eq!(eval_str("base-3", &vars, &f).unwrap(), 7.0);
        // a hyphenated name is not a valid identifier token
        assert!(matches!(parse_expr("a-b-c"), Ok(_))); // parses as a - b - c
        assert_eq!(
            eval_str("a-b", &no_vars(), &f),
            Err(CalcError::UnknownIdent("a".to_string()))
        );
    }

    #[test]
    fn left_associative() {
        let v = no_vars();
        let f = no_fns();
        assert_eq!(eval_str("10 - 3 - 2", &v, &f).unwrap(), 5.0);
        assert_eq!(eval_str("100 / 5 / 2", &v, &f).unwrap(), 10.0);
    }

    #[test]
    fn identifiers_resolve_innermost_first() {
        let f = no_fns();
        let mut outer = HashMap::new();
        outer.insert("a".to_string(), 1.0);
        outer.insert("b".to_string(), 2.0);
        let mut inner = HashMap::new();
        inner.insert("a".to_string(), 10.0);
        let vars = vec![outer, inner];
        assert_eq!(eval_str("a + b", &vars, &f).unwrap(), 12.0);
    }

    #[test]
    fn unknown_identifier_is_an_error() {
        assert_eq!(
            eval_str("a + 1", &no_vars(), &no_fns()),
            Err(CalcError::UnknownIdent("a".to_string()))
        );
    }

    #[test]
    fn syntax_errors() {
        assert!(matches!(parse_expr("2 +"), Err(CalcError::Syntax(_))));
        assert!(matches!(parse_expr("2 3"), Err(CalcError::Syntax(_))));
        assert!(matches!(parse_expr("(1 + 2"), Err(CalcError::Syntax(_))));
        assert!(matches!(parse_expr("2 * * 3"), Err(CalcError::Syntax(_))));
        assert!(matches!(parse_expr(""), Err(CalcError::Syntax(_))));
    }

    #[test]
    fn division_by_zero_is_rejected() {
        let v = no_vars();
        let f = no_fns();
        assert_eq!(eval_str("1 / 0", &v, &f), Err(CalcError::DivByZero));
        assert_eq!(eval_str("1 / (2 - 2)", &v, &f), Err(CalcError::DivByZero));
        assert_eq!(eval_str("5 % 0", &v, &f), Err(CalcError::DivByZero));
    }

    #[test]
    fn user_functions() {
        let (name, scale) = parse_fn("scale(a, b) = a * b").unwrap();
        assert_eq!(name, "scale");
        let mut fns = HashMap::new();
        fns.insert(name, scale);

        let v = no_vars();
        assert_eq!(eval_str("scale(3, 4)", &v, &fns).unwrap(), 12.0);
        assert_eq!(eval_str("scale(scale(1, 1), 3)", &v, &fns).unwrap(), 3.0);
        assert_eq!(eval_str("scale(2, 2) + 1", &v, &fns).unwrap(), 5.0);
    }

    #[test]
    fn identifiers_and_calls_are_case_sensitive() {
        // Names are interned verbatim - `Scale` and `scale` are different.
        let (name, f) = parse_fn("Scale(A, b) = A * b").unwrap();
        assert_eq!(name, "Scale");
        let mut fns = HashMap::new();
        fns.insert(name, f);
        let mut frame = HashMap::new();
        frame.insert("gap".to_string(), 4.0);
        let vars = vec![frame];
        // Exact spelling resolves.
        assert_eq!(eval_str("Scale(gap, 2)", &vars, &fns).unwrap(), 8.0);
        // A different case is an unknown identifier / function.
        assert!(matches!(
            eval_str("SCALE(gap, 2)", &vars, &fns),
            Err(CalcError::UnknownFn(_))
        ));
        assert!(matches!(
            eval_str("Scale(Gap, 2)", &vars, &fns),
            Err(CalcError::UnknownIdent(_))
        ));
    }

    #[test]
    fn function_calling_function() {
        let mut fns = HashMap::new();
        let (n1, f1) = parse_fn("double(x) = x * 2").unwrap();
        let (n2, f2) = parse_fn("quad(x) = double(double(x))").unwrap();
        fns.insert(n1, f1);
        fns.insert(n2, f2);
        assert_eq!(eval_str("quad(3)", &no_vars(), &fns).unwrap(), 12.0);
    }

    #[test]
    fn arity_mismatch() {
        let (name, f) = parse_fn("add(a, b) = a + b").unwrap();
        let mut fns = HashMap::new();
        fns.insert(name, f);
        assert!(matches!(
            eval_str("add(1)", &no_vars(), &fns),
            Err(CalcError::Arity { expected: 2, got: 1, .. })
        ));
    }

    #[test]
    fn unknown_function() {
        assert_eq!(
            eval_str("nope(1)", &no_vars(), &no_fns()),
            Err(CalcError::UnknownFn("nope".to_string()))
        );
    }

    #[test]
    fn recursion_is_caught() {
        let (name, f) = parse_fn("r(n) = r(n)").unwrap();
        let mut fns = HashMap::new();
        fns.insert(name, f);
        assert_eq!(eval_str("r(1)", &no_vars(), &fns), Err(CalcError::Recursion));
    }

    #[test]
    fn function_body_cannot_see_caller_scope() {
        let (name, f) = parse_fn("f(x) = x + outside").unwrap();
        let mut fns = HashMap::new();
        fns.insert(name, f);
        let mut frame = HashMap::new();
        frame.insert("outside".to_string(), 100.0);
        let vars = vec![frame];
        // `outside` is in the caller's scope but not a parameter of `f`.
        assert_eq!(
            eval_str("f(1)", &vars, &fns),
            Err(CalcError::UnknownIdent("outside".to_string()))
        );
    }

    #[test]
    fn parse_fn_rejects_malformed() {
        assert!(parse_fn("scale(a, b) a * b").is_err()); // no `=`
        assert!(parse_fn("scale a, b = a").is_err()); // no parens
        assert!(parse_fn("2bad(a) = a").is_err()); // bad name
        assert!(parse_fn("f(1) = 1").is_err()); // bad param
    }

    #[test]
    fn parse_fn_zero_params() {
        let (name, f) = parse_fn("answer() = 6 * 7").unwrap();
        assert_eq!(name, "answer");
        let mut fns = HashMap::new();
        fns.insert(name, f);
        assert_eq!(eval_str("answer()", &no_vars(), &fns).unwrap(), 42.0);
    }
}
