//! The tiered billing-expression engine.
//!
//! NewAPI's modern pricing path is **not** the classic ratio multiplication. An
//! operator writes a small expression whose coefficients are **USD per million
//! tokens**, and the engine evaluates it against the request's token counts:
//!
//! ```text
//! tier("base", p * 3 + c * 12 + cr * 0.06)
//!           \_____/ \___________/
//!            tier     USD / 1M tokens
//!
//! quota = round( expression_USD / 1_000_000 * QuotaPerUnit * group_ratio )
//! ```
//!
//! Ported from `pkg/billingexpr` (`run.go`, `settle.go`) and
//! `service/tiered_settle.go`'s `BuildTieredTokenParams`.
//!
//! ## The auto-exclusion rule
//!
//! `p` and `c` are *pricing* quantities, not raw counts. If an expression
//! mentions a separately-priced sub-category, that category is subtracted from
//! `p` (or `c`) so it is not charged twice. This is what makes
//! `p * 3 + cr * 0.06` mean "uncached input at $3, cached input at $0.06"
//! rather than charging the cached tokens at both rates.
//!
//! `len` is exempt: it always reports the full input context length so tier
//! conditions keep working regardless of which sub-categories are priced.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::HashMap;

use crate::quota_math::{quota_round_checked, QuotaClamp};
use crate::usage::{BillingUsage, UsageSemantic};

/// `nil`, represented as NaN so comparisons against it are false.
///
/// Go's expression engine yields `nil` for a missing `param(...)`/`header(...)`,
/// and `nil == true` is false. NaN reproduces that for every comparison while
/// keeping the evaluator single-typed.
const NIL: f64 = f64::NAN;

#[derive(Debug, thiserror::Error)]
pub enum ExprError {
    #[error("empty expression")]
    Empty,
    #[error("parse error at {pos}: {message}")]
    Parse { pos: usize, message: String },
    #[error("unknown identifier `{0}`")]
    UnknownIdentifier(String),
    #[error("division by zero")]
    DivisionByZero,
    #[error("unterminated {0}")]
    Unterminated(&'static str),
    #[error("unknown function `{0}`")]
    UnknownFunction(String),
    #[error("`{0}` expects {1} argument(s)")]
    Arity(&'static str, usize),
}

/// Inputs available to an expression beyond the token counts.
///
/// Anything absent evaluates to `nil`, matching NewAPI where a missing body or
/// header produces `nil` rather than an error.
#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    /// Overrides "now" for `hour`/`minute`/`weekday`/`month`/`day` (Unix seconds).
    /// `None` uses the system clock.
    pub now_unix: Option<i64>,
    pub headers: HashMap<String, String>,
    pub body: serde_json::Value,
    /// Extra named usage counters addressable as `u("name")`.
    pub usage_extras: HashMap<String, f64>,
    /// Image count override for `image_count`.
    pub image_count: Option<i64>,
}

/// The evaluation environment: one request's token counts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenParams {
    /// Input tokens available for pricing at the base rate.
    pub p: f64,
    /// Output tokens available for pricing at the base rate.
    pub c: f64,
    /// Full input context length (never reduced by exclusion).
    pub len: f64,
    pub cr: f64,
    pub cc: f64,
    pub cc1h: f64,
    pub img: f64,
    pub img_cr: f64,
    pub img_o: f64,
    pub ai: f64,
    pub ao: f64,
}

/// The result of evaluating an expression.
#[derive(Debug, Clone, PartialEq)]
pub struct ExprOutcome {
    /// The raw expression value: USD for v1 token pricing.
    pub cost_usd: f64,
    /// The tier name recorded by `tier(...)`.
    pub matched_tier: Option<String>,
    /// True when the selected leaf used `fixed(...)`, i.e. per-request pricing.
    pub billing_unit_request: bool,
    /// The per-request USD amount when `billing_unit_request` is set.
    pub fixed_price: Option<f64>,
}

/// Identifiers an expression may reference.
pub const KNOWN_VARS: &[&str] = &[
    "p", "c", "len", "cr", "cc", "cc1h", "img", "img_cr", "img_o", "ai", "ao", "image_count",
];

/// Which token variables an expression references.
///
/// Drives the auto-exclusion subtraction. Computed structurally so a variable
/// appearing only in a branch that is not taken still counts, matching NewAPI's
/// AST-based `UsedVarsByHash`.
pub fn used_vars(expression: &str) -> HashMap<String, bool> {
    let mut out = HashMap::new();
    let bytes = expression.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c.is_ascii_alphanumeric() || c == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let word = &expression[start..i];
            if KNOWN_VARS.contains(&word) {
                out.insert(word.to_string(), true);
            }
            continue;
        }
        i += 1;
    }
    out
}

/// Build the evaluation environment from usage, applying auto-exclusion.
///
/// Ported from `BuildTieredTokenParams`.
pub fn build_params(usage: &BillingUsage, used: &HashMap<String, bool>) -> TokenParams {
    let is_claude = usage.semantic == UsageSemantic::Anthropic;

    let mut p = usage.prompt_tokens as f64;
    let mut c = usage.completion_tokens as f64;
    let cr = usage.cached_tokens as f64;

    let (cc5m, cc1h) = if is_claude {
        (
            usage.cache_creation_5m_tokens as f64,
            usage.cache_creation_1h_tokens as f64,
        )
    } else {
        (usage.cache_creation_tokens as f64, 0.0)
    };

    let img = usage.image_tokens as f64;
    let img_cr = 0.0f64;
    let ai = usage.audio_tokens as f64;
    let img_o = 0.0f64;
    let ao = 0.0f64;

    // `len` is the full input context, before any exclusion.
    let input_len = if is_claude { p + cr + cc5m + cc1h } else { p };

    if is_claude {
        // Anthropic's input_tokens exclude cache reads, so when the expression
        // has no separate cache price the reads must be folded back in.
        if !used.contains_key("cr") {
            p += cr;
        }
    } else {
        if used.contains_key("cr") {
            p -= cr;
        }
        if used.contains_key("cc") {
            p -= cc5m;
        }
        if used.contains_key("cc1h") {
            p -= cc1h;
        }
        if used.contains_key("img") {
            p -= img;
        }
        if used.contains_key("img_cr") {
            p -= img_cr;
        }
        if used.contains_key("ai") {
            p -= ai;
        }
        if used.contains_key("img_o") {
            c -= img_o;
        }
        if used.contains_key("ao") {
            c -= ao;
        }
    }

    // OpenAI cache-write usage reports unadjusted prefix counts, so `cr + cc`
    // can exceed the prompt and drive the remainder negative. Clamp at zero.
    if p < 0.0 {
        p = 0.0;
    }
    if c < 0.0 {
        c = 0.0;
    }

    TokenParams {
        p,
        c,
        len: input_len,
        cr,
        cc: cc5m,
        cc1h,
        img,
        img_cr,
        img_o,
        ai,
        ao,
    }
}

/// Convert an evaluated expression result to quota.
///
/// v1 semantics: the expression yields USD, so scale by `QuotaPerUnit`, apply
/// the group ratio, then round half-away-from-zero with int32 saturation.
pub fn quota_from_cost(
    cost_usd: f64,
    quota_per_unit: f64,
    group_ratio: f64,
) -> (i64, Option<QuotaClamp>) {
    let before_group = cost_usd / 1_000_000.0 * quota_per_unit;
    quota_round_checked(before_group * group_ratio)
}

/// Evaluate `expression` against `params`.
pub fn evaluate(expression: &str, params: &TokenParams) -> Result<ExprOutcome, ExprError> {
    evaluate_with(expression, params, &EvalContext::default())
}

/// Evaluate `expression` with request context available to `param`/`header`/`u`
/// and the time functions.
pub fn evaluate_with(
    expression: &str,
    params: &TokenParams,
    ctx: &EvalContext,
) -> Result<ExprOutcome, ExprError> {
    let expr = expression.trim();
    if expr.is_empty() {
        return Err(ExprError::Empty);
    }
    let mut parser = Parser::new(expr);
    let node = parser.parse_expression()?;
    parser.skip_ws();
    if !parser.at_end() {
        return Err(ExprError::Parse {
            pos: parser.pos,
            message: "trailing input".to_string(),
        });
    }

    let mut trace = Trace::default();
    let value = eval_node(&node, params, ctx, &mut trace)?;
    Ok(ExprOutcome {
        cost_usd: value,
        matched_tier: trace.matched_tier,
        billing_unit_request: trace.billing_unit_request,
        fixed_price: trace.fixed_price,
    })
}

// ---------------------------------------------------------------------------
// AST and parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Number(f64),
    Var(String),
    Str(String),
    Unary(UnaryOp, Box<Node>),
    Binary(BinaryOp, Box<Node>, Box<Node>),
    Ternary(Box<Node>, Box<Node>, Box<Node>),
    Call(String, Vec<Node>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Default)]
struct Trace {
    matched_tier: Option<String>,
    billing_unit_request: bool,
    fixed_price: Option<f64>,
}

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.bytes.get(self.pos).map(|b| *b as char)
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s)
    }

    fn eat(&mut self, s: &str) -> bool {
        self.skip_ws();
        if self.starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn parse_expression(&mut self) -> Result<Node, ExprError> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Result<Node, ExprError> {
        let cond = self.parse_binary(0)?;
        self.skip_ws();
        if self.eat("?") {
            let then_branch = self.parse_ternary()?;
            if !self.eat(":") {
                return Err(ExprError::Parse {
                    pos: self.pos,
                    message: "expected `:` in conditional".to_string(),
                });
            }
            let else_branch = self.parse_ternary()?;
            return Ok(Node::Ternary(
                Box::new(cond),
                Box::new(then_branch),
                Box::new(else_branch),
            ));
        }
        Ok(cond)
    }

    fn parse_binary(&mut self, min_prec: u8) -> Result<Node, ExprError> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_ws();
            let Some((op, prec, len)) = self.peek_binary_op() else {
                break;
            };
            if prec < min_prec {
                break;
            }
            self.pos += len;
            // Left-associative: the right side binds tighter.
            let right = self.parse_binary(prec + 1)?;
            left = Node::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Longest-match operator table. Word operators need a boundary so
    /// `android` is not read as `and`.
    fn peek_binary_op(&self) -> Option<(BinaryOp, u8, usize)> {
        let rest = &self.src[self.pos..];
        let table: &[(&str, BinaryOp, u8)] = &[
            ("&&", BinaryOp::And, 1),
            ("||", BinaryOp::Or, 1),
            ("==", BinaryOp::Eq, 3),
            ("!=", BinaryOp::Ne, 3),
            ("<=", BinaryOp::Le, 4),
            (">=", BinaryOp::Ge, 4),
            ("<", BinaryOp::Lt, 4),
            (">", BinaryOp::Gt, 4),
            ("+", BinaryOp::Add, 5),
            ("-", BinaryOp::Sub, 5),
            ("*", BinaryOp::Mul, 6),
            ("/", BinaryOp::Div, 6),
            ("%", BinaryOp::Mod, 6),
        ];
        for (text, op, prec) in table {
            if rest.starts_with(text) {
                return Some((*op, *prec, text.len()));
            }
        }
        for (text, op) in [("and", BinaryOp::And), ("or", BinaryOp::Or)] {
            if rest.starts_with(text) {
                let after = rest.as_bytes().get(text.len()).copied();
                let boundary = match after {
                    None => true,
                    Some(c) => !(c as char).is_ascii_alphanumeric() && c != b'_',
                };
                if boundary {
                    return Some((op, 1, text.len()));
                }
            }
        }
        None
    }

    fn parse_unary(&mut self) -> Result<Node, ExprError> {
        self.skip_ws();
        if self.eat("!") {
            let inner = self.parse_unary()?;
            return Ok(Node::Unary(UnaryOp::Not, Box::new(inner)));
        }
        if self.eat("-") {
            let inner = self.parse_unary()?;
            return Ok(Node::Unary(UnaryOp::Neg, Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Node, ExprError> {
        self.skip_ws();
        if self.at_end() {
            return Err(ExprError::Parse {
                pos: self.pos,
                message: "unexpected end of expression".to_string(),
            });
        }
        if self.eat("(") {
            let inner = self.parse_expression()?;
            if !self.eat(")") {
                return Err(ExprError::Parse {
                    pos: self.pos,
                    message: "expected `)`".to_string(),
                });
            }
            return Ok(inner);
        }
        let c = self.peek().unwrap_or(' ');
        if c == '"' || c == '\'' {
            return self.parse_string();
        }
        if c.is_ascii_digit() || c == '.' {
            return self.parse_number();
        }
        if c.is_ascii_alphabetic() || c == '_' {
            return self.parse_identifier();
        }
        Err(ExprError::Parse {
            pos: self.pos,
            message: format!("unexpected character `{}`", c),
        })
    }

    fn parse_string(&mut self) -> Result<Node, ExprError> {
        let quote = self.peek().unwrap_or('"');
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos] as char != quote {
            self.pos += 1;
        }
        if self.at_end() {
            return Err(ExprError::Unterminated("string literal"));
        }
        let text = self.src[start..self.pos].to_string();
        self.pos += 1;
        Ok(Node::Str(text))
    }

    fn parse_number(&mut self) -> Result<Node, ExprError> {
        let start = self.pos;
        let mut seen_dot = false;
        let mut seen_exp = false;
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos] as char;
            if c.is_ascii_digit() {
                self.pos += 1;
            } else if c == '.' && !seen_dot && !seen_exp {
                seen_dot = true;
                self.pos += 1;
            } else if (c == 'e' || c == 'E') && !seen_exp {
                seen_exp = true;
                self.pos += 1;
                if self.pos < self.bytes.len() {
                    let sign = self.bytes[self.pos] as char;
                    if sign == '+' || sign == '-' {
                        self.pos += 1;
                    }
                }
            } else {
                break;
            }
        }
        let text = &self.src[start..self.pos];
        text.parse::<f64>().map(Node::Number).map_err(|_| ExprError::Parse {
            pos: start,
            message: format!("invalid number `{}`", text),
        })
    }

    fn parse_identifier(&mut self) -> Result<Node, ExprError> {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos] as char;
            if c.is_ascii_alphanumeric() || c == '_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let name = self.src[start..self.pos].to_string();

        match name.as_str() {
            "true" => return Ok(Node::Number(1.0)),
            "false" => return Ok(Node::Number(0.0)),
            _ => {}
        }

        self.skip_ws();
        if self.peek() == Some('(') {
            self.pos += 1;
            let mut args = Vec::new();
            self.skip_ws();
            if self.peek() == Some(')') {
                self.pos += 1;
                return Ok(Node::Call(name, args));
            }
            loop {
                args.push(self.parse_expression()?);
                self.skip_ws();
                if self.eat(",") {
                    continue;
                }
                if self.eat(")") {
                    break;
                }
                return Err(ExprError::Parse {
                    pos: self.pos,
                    message: "expected `,` or `)` in argument list".to_string(),
                });
            }
            return Ok(Node::Call(name, args));
        }

        if KNOWN_VARS.contains(&name.as_str()) {
            return Ok(Node::Var(name));
        }
        Err(ExprError::UnknownIdentifier(name))
    }
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

fn eval_node(node: &Node, params: &TokenParams, ctx: &EvalContext, trace: &mut Trace) -> Result<f64, ExprError> {
    match node {
        Node::Number(n) => Ok(*n),
        Node::Str(_) => Ok(0.0),
        Node::Var(name) => Ok(match name.as_str() {
            "p" => params.p,
            "c" => params.c,
            "len" => params.len,
            "cr" => params.cr,
            "cc" => params.cc,
            "cc1h" => params.cc1h,
            "img" => params.img,
            "img_cr" => params.img_cr,
            "img_o" => params.img_o,
            "ai" => params.ai,
            "ao" => params.ao,
            "image_count" => 1.0,
            other => return Err(ExprError::UnknownIdentifier(other.to_string())),
        }),
        Node::Unary(op, inner) => {
            let value = eval_node(inner, params, ctx, trace)?;
            Ok(match op {
                UnaryOp::Neg => -value,
                UnaryOp::Not => {
                    if value == 0.0 {
                        1.0
                    } else {
                        0.0
                    }
                }
            })
        }
        Node::Ternary(cond, then_branch, else_branch) => {
            let test = eval_node(cond, params, ctx, trace)?;
            if test != 0.0 {
                eval_node(then_branch, params, ctx, trace)
            } else {
                eval_node(else_branch, params, ctx, trace)
            }
        }
        Node::Binary(op, left, right) => {
            match op {
                BinaryOp::And => {
                    let l = eval_node(left, params, ctx, trace)?;
                    if l == 0.0 {
                        return Ok(0.0);
                    }
                    let r = eval_node(right, params, ctx, trace)?;
                    return Ok(if r != 0.0 { 1.0 } else { 0.0 });
                }
                BinaryOp::Or => {
                    let l = eval_node(left, params, ctx, trace)?;
                    if l != 0.0 {
                        return Ok(1.0);
                    }
                    let r = eval_node(right, params, ctx, trace)?;
                    return Ok(if r != 0.0 { 1.0 } else { 0.0 });
                }
                _ => {}
            }

            let l = eval_node(left, params, ctx, trace)?;
            let r = eval_node(right, params, ctx, trace)?;
            let value = match op {
                BinaryOp::Add => l + r,
                BinaryOp::Sub => l - r,
                BinaryOp::Mul => l * r,
                BinaryOp::Div => {
                    if r == 0.0 {
                        return Err(ExprError::DivisionByZero);
                    }
                    l / r
                }
                BinaryOp::Mod => {
                    if r == 0.0 {
                        return Err(ExprError::DivisionByZero);
                    }
                    l % r
                }
                BinaryOp::Lt => bool_num(l < r),
                BinaryOp::Le => bool_num(l <= r),
                BinaryOp::Gt => bool_num(l > r),
                BinaryOp::Ge => bool_num(l >= r),
                BinaryOp::Eq => bool_num(l == r),
                BinaryOp::Ne => bool_num(l != r),
                BinaryOp::And | BinaryOp::Or => unreachable!("handled above"),
            };
            Ok(value)
        }
        Node::Call(name, args) => eval_call(name, args, params, ctx, trace),
    }
}

fn eval_call(
    name: &str,
    args: &[Node],
    params: &TokenParams,
    ctx: &EvalContext,
    trace: &mut Trace,
) -> Result<f64, ExprError> {
    match name {
        "tier" => {
            if args.len() != 2 {
                return Err(ExprError::Arity("tier", 2));
            }
            // The tier label is a literal in every expression the live instance
            // uses; fall back to evaluating it when it is computed.
            let tier_name = match &args[0] {
                Node::Str(s) => s.clone(),
                Node::Var(v) => v.clone(),
                other => eval_node(other, params, ctx, trace)?.to_string(),
            };
            let value = eval_node(&args[1], params, ctx, trace)?;
            trace.matched_tier = Some(tier_name);
            Ok(value)
        }
        "fixed" => {
            if args.len() != 1 {
                return Err(ExprError::Arity("fixed", 1));
            }
            let amount = eval_node(&args[0], params, ctx, trace)?;
            trace.billing_unit_request = true;
            trace.fixed_price = Some(amount);
            // Mirrors NewAPI: returns `amount * 1_000_000` so the v1
            // `/ 1_000_000` conversion yields exactly the USD amount.
            Ok(amount * 1_000_000.0)
        }
        "max" | "min" => {
            if args.len() != 2 {
                return Err(ExprError::Arity(
                    if name == "max" { "max" } else { "min" },
                    2,
                ));
            }
            let a = eval_node(&args[0], params, ctx, trace)?;
            let b = eval_node(&args[1], params, ctx, trace)?;
            Ok(if name == "max" { a.max(b) } else { a.min(b) })
        }
        "abs" | "ceil" | "floor" => {
            if args.len() != 1 {
                return Err(ExprError::Arity(
                    match name {
                        "abs" => "abs",
                        "ceil" => "ceil",
                        _ => "floor",
                    },
                    1,
                ));
            }
            let a = eval_node(&args[0], params, ctx, trace)?;
            Ok(match name {
                "abs" => a.abs(),
                "ceil" => a.ceil(),
                _ => a.floor(),
            })
        }
        "param" => {
            if args.len() != 1 {
                return Err(ExprError::Arity("param", 1));
            }
            let path = match &args[0] {
                Node::Str(s) => s.clone(),
                other => eval_node(other, params, ctx, trace)?.to_string(),
            };
            // `param(...) == true` is how operators gate on a request flag; a
            // missing path or a non-true value both mean "not enabled".
            let found = json_path(&ctx.body, &path);
            Ok(match found {
                Some(serde_json::Value::Bool(true)) => 1.0,
                Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(NIL),
                Some(serde_json::Value::String(s)) => {
                    if s.eq_ignore_ascii_case("true") {
                        1.0
                    } else if s.eq_ignore_ascii_case("false") {
                        0.0
                    } else {
                        NIL
                    }
                }
                _ => NIL,
            })
        }
        "header" => {
            if args.len() != 1 {
                return Err(ExprError::Arity("header", 1));
            }
            let key = match &args[0] {
                Node::Str(s) => s.clone(),
                other => eval_node(other, params, ctx, trace)?.to_string(),
            };
            let key = key.trim().to_ascii_lowercase();
            match ctx
                .headers
                .iter()
                .find(|(k, _)| k.trim().to_ascii_lowercase() == key)
                .map(|(_, v)| v.trim().to_string())
            {
                Some(v) => Ok(v.parse::<f64>().unwrap_or(NIL)),
                None => Ok(NIL),
            }
        }
        "u" => {
            if args.len() != 1 {
                return Err(ExprError::Arity("u", 1));
            }
            let key = match &args[0] {
                Node::Str(s) => s.trim().to_string(),
                other => eval_node(other, params, ctx, trace)?.to_string(),
            };
            Ok(ctx.usage_extras.get(&key).copied().unwrap_or(NIL))
        }
        "has" => {
            if args.len() != 2 {
                return Err(ExprError::Arity("has", 2));
            }
            let source = match &args[0] {
                Node::Str(s) => Some(s.clone()),
                other => eval_node(other, params, ctx, trace)
                    .ok()
                    .map(|v| v.to_string()),
            };
            let needle = match &args[1] {
                Node::Str(s) => s.clone(),
                other => eval_node(other, params, ctx, trace)?.to_string(),
            };
            match source {
                Some(s) if !needle.is_empty() => Ok(bool_num(s.contains(&needle))),
                _ => Ok(0.0),
            }
        }
        "hour" | "minute" | "weekday" | "month" | "day" => {
            if args.len() != 1 {
                return Err(ExprError::Arity(
                    match name {
                        "hour" => "hour",
                        "minute" => "minute",
                        "weekday" => "weekday",
                        "month" => "month",
                        _ => "day",
                    },
                    1,
                ));
            }
            let tz = match &args[0] {
                Node::Str(s) => s.trim().to_string(),
                other => eval_node(other, params, ctx, trace)?.to_string(),
            };
            let (year, month, day, hour, minute, weekday) =
                local_time_parts(ctx.now_unix, &tz);
            // Calendar parts, already bounded by construction.
            let value = match name {
                "hour" => hour,
                "minute" => minute,
                "weekday" => weekday,
                "month" => month,
                _ => day,
            };
            let _ = year;
            Ok(value as f64)
        }        other => Err(ExprError::UnknownFunction(other.to_string())),
    }
}

/// Minimal JSON-path lookup supporting the dotted form operators use
/// (`enable_thinking`, `metadata.user_id`).
fn json_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return None;
    }
    let mut current = root;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = match current {
            serde_json::Value::Object(map) => map.get(segment)?,
            serde_json::Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Convert a Unix timestamp to local calendar parts.
///
/// Hand-written so the engine carries no timezone database dependency, and so
/// evaluation is deterministic when `now_unix` is pinned by a test.
fn local_time_parts(now_unix: Option<i64>, tz: &str) -> (i32, u32, u32, u32, u32, u32) {
    let ts = now_unix.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });
    let offset_seconds = tz_offset_seconds(tz);
    let shifted = ts + offset_seconds;

    let days = shifted.div_euclid(86_400);
    let secs_of_day = shifted.rem_euclid(86_400);
    let hour = (secs_of_day / 3600) as u32;
    let minute = ((secs_of_day % 3600) / 60) as u32;

    // 1970-01-01 was a Thursday, so weekday 0 (Sunday) is day 4.
    let weekday = ((days + 4).rem_euclid(7)) as u32;
    let (year, month, day) = civil_from_days(days);
    (year, month, day, hour, minute, weekday)
}

/// Fixed UTC offsets for the zones operators actually use; unknown zones are
/// treated as UTC rather than failing the charge.
fn tz_offset_seconds(tz: &str) -> i64 {
    match tz.trim().to_ascii_uppercase().as_str() {
        "" | "UTC" | "GMT" | "ETC/UTC" => 0,
        "ASIA/SHANGHAI" | "ASIA/CHONGQING" | "ASIA/URUMQI" | "PRC" | "CTT" => 8 * 3600,
        "ASIA/TOKYO" | "JST" | "ASIA/SEOUL" => 9 * 3600,
        "ASIA/SINGAPORE" | "ASIA/HONG_KONG" | "HONGKONG" => 8 * 3600,
        "ASIA/KOLKATA" | "ASIA/CALCUTTA" | "IST" => 5 * 3600 + 1800,
        "AMERICA/NEW_YORK" | "EST5EDT" | "US/EASTERN" => -5 * 3600,
        "AMERICA/CHICAGO" | "CST6CDT" | "US/CENTRAL" => -6 * 3600,
        "AMERICA/DENVER" | "MST7MDT" | "US/MOUNTAIN" => -7 * 3600,
        "AMERICA/LOS_ANGELES" | "PST8PDT" | "US/PACIFIC" => -8 * 3600,
        "EUROPE/LONDON" | "GB" => 0,
        "EUROPE/BERLIN" | "EUROPE/PARIS" | "EUROPE/ROME" | "CET" => 3600,
        "EUROPE/MOSCOW" => 3 * 3600,
        "AUSTRALIA/SYDNEY" => 10 * 3600,
        _ => 0,
    }
}

/// Days since the Unix epoch to a civil date. Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m, d)
}

fn bool_num(condition: bool) -> f64 {
    if condition {
        1.0
    } else {
        0.0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::BillingUsage;

    fn openai_usage(prompt: i64, completion: i64, cached: i64) -> BillingUsage {
        BillingUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            cached_tokens: cached,
            semantic: UsageSemantic::OpenAi,
            ..Default::default()
        }
    }

    fn run(expr: &str, usage: &BillingUsage) -> ExprOutcome {
        let used = used_vars(expr);
        let params = build_params(usage, &used);
        evaluate(expr, &params).expect("expression should evaluate")
    }

    #[test]
    fn the_live_tiered_expression_reproduces_the_recorded_charge() {
        // Exactly the expression the live instance stores for glm-5.3-flash.
        let expr = r#"tier("base", p * 1 + c * 3 + cr * 0.3)"#;
        let out = run(expr, &openai_usage(569_944, 336, 568_768));
        assert_eq!(out.matched_tier.as_deref(), Some("base"));

        // p_used = 569944 - 568768 = 1176
        // 1176*1 + 336*3 + 568768*0.3 = 1176 + 1008 + 170630.4 = 172814.4
        assert!((out.cost_usd - 172_814.4).abs() < 1e-6, "got {}", out.cost_usd);

        let (quota, clamp) = quota_from_cost(out.cost_usd, 500_000.0, 10.0);
        assert!(clamp.is_none());
        // round(172814.4 / 1e6 * 500000 * 10) = round(864072) = 864072
        assert_eq!(quota, 864_072);
        assert_eq!(quota, 864_072u32 as i64);
    }

    #[test]
    fn the_other_live_expression_reproduces_its_charge() {
        // tier("base", p * 3 + c * 12 + cr * 0.06)
        let expr = r#"tier("base", p * 3 + c * 12 + cr * 0.06)"#;
        let out = run(expr, &openai_usage(506_662, 787, 503_424));
        // p_used = 506662 - 503424 = 3238
        let expected = 3238.0 * 3.0 + 787.0 * 12.0 + 503_424.0 * 0.06;
        assert!((out.cost_usd - expected).abs() < 1e-6);
        let (quota, _) = quota_from_cost(out.cost_usd, 500_000.0, 10.0);
        assert_eq!(quota, 246_817);
    }

    #[test]
    fn auto_exclusion_only_applies_to_referenced_vars() {
        let usage = openai_usage(1000, 100, 400);

        // No `cr`: the 400 cached tokens stay inside `p`.
        let without = run(r#"tier("base", p * 3)"#, &usage);
        assert!((without.cost_usd - 3000.0).abs() < 1e-9);

        // With `cr`: 400 leaves `p` and is charged at its own rate.
        let with = run(r#"tier("base", p * 3 + cr * 0.5)"#, &usage);
        // 600*3 + 400*0.5 = 1800 + 200 = 2000
        assert!((with.cost_usd - 2000.0).abs() < 1e-9);
    }

    #[test]
    fn len_is_never_reduced_by_exclusion() {
        let usage = openai_usage(1000, 0, 400);
        let used = used_vars(r#"tier("base", p * 1 + cr * 1)"#);
        let params = build_params(&usage, &used);
        assert_eq!(params.p, 600.0);
        assert_eq!(params.len, 1000.0, "len must stay the full context length");
    }

    #[test]
    fn claude_usage_folds_cache_reads_into_p_when_unpriced() {
        let usage = BillingUsage {
            prompt_tokens: 1000,
            cached_tokens: 400,
            semantic: UsageSemantic::Anthropic,
            ..Default::default()
        };
        let used = used_vars(r#"tier("base", p * 3)"#);
        let params = build_params(&usage, &used);
        assert_eq!(params.p, 1400.0);
    }

    #[test]
    fn claude_len_includes_cache_terms() {
        let usage = BillingUsage {
            prompt_tokens: 1000,
            cached_tokens: 400,
            cache_creation_5m_tokens: 100,
            semantic: UsageSemantic::Anthropic,
            ..Default::default()
        };
        let used = used_vars(r#"tier("base", p * 1)"#);
        let params = build_params(&usage, &used);
        assert_eq!(params.len, 1500.0);
    }

    #[test]
    fn negative_prompt_is_clamped() {
        let usage = BillingUsage {
            prompt_tokens: 100,
            cached_tokens: 80,
            cache_creation_tokens: 80,
            semantic: UsageSemantic::OpenAi,
            ..Default::default()
        };
        let used = used_vars(r#"tier("base", p * 1 + cr * 1 + cc * 1)"#);
        let params = build_params(&usage, &used);
        assert_eq!(params.p, 0.0);
    }

    #[test]
    fn conditional_tier_selects_long_context_rates() {
        let expr = r#"len <= 200000 ? tier("standard", p * 3 + c * 15) : tier("long_context", p * 6 + c * 22.5)"#;

        let short = run(expr, &openai_usage(1000, 100, 0));
        assert_eq!(short.matched_tier.as_deref(), Some("standard"));
        assert!((short.cost_usd - (1000.0 * 3.0 + 100.0 * 15.0)).abs() < 1e-9);

        let long = run(expr, &openai_usage(300_000, 100, 0));
        assert_eq!(long.matched_tier.as_deref(), Some("long_context"));
        assert!((long.cost_usd - (300_000.0 * 6.0 + 100.0 * 22.5)).abs() < 1e-9);
    }

    #[test]
    fn fixed_price_is_per_request() {
        let expr = r#"len <= 32000 ? tier("short", fixed(0.01)) : tier("long", p * 2 + c * 8)"#;

        let short = run(expr, &openai_usage(100, 10, 0));
        assert!(short.billing_unit_request);
        assert_eq!(short.fixed_price, Some(0.01));

        let (quota, _) = quota_from_cost(short.cost_usd, 500_000.0, 1.0);
        assert_eq!(quota, 5_000); // $0.01 * 500_000

        let long = run(expr, &openai_usage(100_000, 10, 0));
        assert!(!long.billing_unit_request);
    }

    #[test]
    fn all_builtin_expressions_from_the_live_instance_parse() {
        // The two expressions covering every tiered_expr request in the live DB.
        for expr in [
            r#"tier("base", p * 3 + c * 12 + cr * 0.06)"#,
            r#"tier("base", p * 1 + c * 3 + cr * 0.3)"#,
        ] {
            let out = run(expr, &openai_usage(1000, 100, 200));
            assert!(out.matched_tier.is_some(), "failed for {expr}");
        }
    }

    #[test]
    fn documented_examples_parse() {
        for expr in [
            r#"tier("base", p * 2.5 + c * 15 + cr * 0.25)"#,
            r#"len <= 200000 ? tier("standard", p * 3 + c * 15 + cr * 0.3 + cc * 3.75 + cc1h * 6) : tier("long_context", p * 6 + c * 22.5 + cr * 0.6 + cc * 7.5 + cc1h * 12)"#,
            r#"tier("base", p * 2 + c * 8 + img * 2.5)"#,
            r#"tier("base", p * 0.43 + c * 3.06 + img * 0.78 + ai * 3.81 + ao * 15.11)"#,
            r#"tier("standard", p * 5 + cr * 1.25 + img * 8 + img_cr * 2 + c * 30)"#,
            r#"len <= 272000 ? tier("standard", p * 10 + c * 50 + cr * 1 + cc * 12.5) : tier("long_context", p * 20 + c * 75 + cr * 2 + cc * 25)"#,
        ] {
            let used = used_vars(expr);
            let params = build_params(&openai_usage(1000, 100, 0), &used);
            assert!(evaluate(expr, &params).is_ok(), "failed for {expr}");
        }
    }

    #[test]
    fn math_helpers_work() {
        let zero = openai_usage(0, 0, 0);
        assert!((run(r#"tier("t", max(3, 7))"#, &zero).cost_usd - 7.0).abs() < 1e-9);
        assert!((run(r#"tier("t", min(3, 7))"#, &zero).cost_usd - 3.0).abs() < 1e-9);
        assert!((run(r#"tier("t", abs(-4))"#, &zero).cost_usd - 4.0).abs() < 1e-9);
        assert!((run(r#"tier("t", ceil(1.2))"#, &zero).cost_usd - 2.0).abs() < 1e-9);
        assert!((run(r#"tier("t", floor(1.8))"#, &zero).cost_usd - 1.0).abs() < 1e-9);
    }

    #[test]
    fn operator_precedence_is_standard() {
        let zero = openai_usage(0, 0, 0);
        assert!((run(r#"tier("t", 2 + 3 * 4)"#, &zero).cost_usd - 14.0).abs() < 1e-9);
        assert!((run(r#"tier("t", (2 + 3) * 4)"#, &zero).cost_usd - 20.0).abs() < 1e-9);
        assert!((run(r#"tier("t", -2 * 3)"#, &zero).cost_usd + 6.0).abs() < 1e-9);
    }

    #[test]
    fn comparison_and_logic_operators() {
        let zero = openai_usage(0, 0, 0);
        assert!((run(r#"tier("t", 1 < 2)"#, &zero).cost_usd - 1.0).abs() < 1e-9);
        assert!((run(r#"tier("t", 2 <= 2)"#, &zero).cost_usd - 1.0).abs() < 1e-9);
        assert!((run(r#"tier("t", 3 > 4)"#, &zero).cost_usd).abs() < 1e-9);
        assert!((run(r#"tier("t", 1 > 0 && 0 > 1)"#, &zero).cost_usd).abs() < 1e-9);
        assert!((run(r#"tier("t", 1 > 0 || 0 > 1)"#, &zero).cost_usd - 1.0).abs() < 1e-9);
    }

    #[test]
    fn division_by_zero_is_reported_not_silent() {
        let params = TokenParams::default();
        assert!(matches!(
            evaluate(r#"tier("t", 1 / 0)"#, &params),
            Err(ExprError::DivisionByZero)
        ));
    }

    #[test]
    fn unknown_identifiers_are_rejected() {
        let params = TokenParams::default();
        assert!(matches!(
            evaluate(r#"tier("t", bogus * 2)"#, &params),
            Err(ExprError::UnknownIdentifier(_))
        ));
    }

    #[test]
    fn trailing_garbage_is_rejected() {
        let params = TokenParams::default();
        assert!(evaluate(r#"tier("t", 1) extra"#, &params).is_err());
    }

    #[test]
    fn unterminated_string_is_reported() {
        let params = TokenParams::default();
        assert!(evaluate(r#"tier("t, 1)"#, &params).is_err());
    }

    #[test]
    fn used_vars_finds_variables_in_dead_branches() {
        // Must be structural: excluded tokens are deducted even when the branch
        // pricing them is not the one taken at settlement.
        let used = used_vars(r#"len <= 10 ? tier("a", p * 1) : tier("b", p * 1 + cr * 0.1)"#);
        assert!(used.contains_key("cr"));
        assert!(used.contains_key("len"));
    }

    #[test]
    fn scientific_notation_parses() {
        let zero = openai_usage(0, 0, 0);
        assert!((run(r#"tier("t", 1e-6 * 1000000)"#, &zero).cost_usd - 1.0).abs() < 1e-9);
    }

    #[test]
    fn zero_and_free_expressions_are_valid() {
        let zero = openai_usage(100, 100, 0);
        let out = run(r#"tier("free", fixed(0))"#, &zero);
        assert_eq!(out.fixed_price, Some(0.0));
        let (quota, _) = quota_from_cost(out.cost_usd, 500_000.0, 10.0);
        assert_eq!(quota, 0);
    }

    #[test]
    fn unknown_function_is_rejected() {
        let params = TokenParams::default();
        assert!(matches!(
            evaluate(r#"bogus(1)"#, &params),
            Err(ExprError::UnknownFunction(_))
        ));
    }
}
