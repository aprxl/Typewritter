//! Numeric meaning for a math tree, for plotting. The tree stays
//! presentational (MATH.md); this module reads one tree as an expression,
//! translates it to exmex syntax and lets exmex parse and evaluate it.
//!
//! The reading is deliberately literal. Every plain letter is a variable of
//! its own (`xy` is `x·y`, as in TeX), juxtaposition multiplies, and only
//! symbols resolved as functions or constants mean one. Anything without a
//! numeric reading yet — sums, accents, user functions — is an
//! [`EvalError::Unsupported`] naming what it met, never a guess.
//!
//! Calculus follows the notation:
//!
//! - A Leibniz fraction — `d/dx`, `∂/∂x`, `d²/dx²`, or `d(…)/dx` — applies
//!   to what follows it like a function. exmex differentiates it
//!   symbolically; a body holding an integral, which exmex cannot see
//!   into, is differentiated numerically instead.
//! - `∫` takes everything up to its own differential (`dt`), nested
//!   integrals closing their own first. With limits it is a definite
//!   integral, in which the limits may use the curve's variable; without,
//!   it is the antiderivative that is zero at 0. exmex does not integrate,
//!   so each integral stands in the exmex text as a placeholder variable
//!   whose value is computed by adaptive Simpson quadrature per evaluation.

use std::collections::BTreeSet;
use std::fmt;
use std::rc::Rc;

use exmex::prelude::*;

use super::math::{BigOp, MathList, MathNode, SymbolRole};

/// Why a tree has no numeric reading.
#[derive(Clone, Debug, PartialEq)]
pub enum EvalError {
    /// Notation this module does not evaluate yet.
    Unsupported(String),
    /// A shape that cannot be read, such as an operator with no operand.
    Malformed(String),
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(what) => write!(f, "not supported yet: {what}"),
            Self::Malformed(what) => write!(f, "cannot read: {what}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// A plottable `y = f(x)`: one output name, at most one input, and the
/// expression between them.
#[derive(Clone, Debug)]
pub struct Curve {
    /// The left-hand variable, when the tree was an equation.
    pub output: Option<String>,
    /// The single free variable, if the expression has one.
    pub input: Option<String>,
    expression: Expr,
}

impl Curve {
    /// Reads `list` as `name = expression` or as a bare expression.
    pub fn parse(list: &MathList) -> Result<Self, EvalError> {
        let equals: Vec<usize> = list
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node, MathNode::Sym('=')))
            .map(|(index, _)| index)
            .collect();
        let (output, body) = match equals.as_slice() {
            [] => (None, list.as_slice()),
            [at] => (Some(output_name(&list[..*at])?), &list[at + 1..]),
            _ => {
                return Err(EvalError::Unsupported(
                    "more than one = in a curve".to_owned(),
                ));
            }
        };
        let mut translator = Translator::default();
        let text = translator.translate(body)?;
        let expression = translator.compile(&text)?;
        let mut free = BTreeSet::new();
        expression.free(&mut free);
        let input = match free.len() {
            0 => None,
            1 => free.pop_first(),
            _ => {
                return Err(EvalError::Unsupported(format!(
                    "a curve in more than one variable ({})",
                    free.into_iter().collect::<Vec<_>>().join(", ")
                )));
            }
        };
        Ok(Self {
            output,
            input,
            expression,
        })
    }

    /// The curve's value at `x`; NaN where it is undefined.
    pub fn eval(&self, x: f64) -> f64 {
        let mut scope = Vec::new();
        if let Some(input) = &self.input {
            scope.push((input.as_str(), x));
        }
        self.expression.eval(&mut scope)
    }

    /// The exmex form of the top-level expression, for diagnostics.
    /// Integrals and numeric derivatives appear as `{⟨n⟩}` placeholders.
    pub fn expression(&self) -> &str {
        self.expression.flat.unparse()
    }
}

/// Variable bindings, innermost last: the curve's input, then any
/// integration variable or derivative point being evaluated inside it.
type Scope<'a> = Vec<(&'a str, f64)>;

fn lookup(scope: &Scope, name: &str) -> f64 {
    scope
        .iter()
        .rev()
        .find(|(bound, _)| *bound == name)
        .map_or(f64::NAN, |(_, value)| *value)
}

/// A parsed exmex expression and where each of its variables' values
/// comes from, in exmex's variable order.
#[derive(Clone, Debug)]
struct Expr {
    flat: FlatEx<f64>,
    sources: Vec<Source>,
}

#[derive(Clone, Debug)]
enum Source {
    /// A variable the scope binds.
    Variable(String),
    Integral(Rc<Integral>),
    Derivative(Rc<Derivative>),
}

impl Expr {
    fn eval<'a>(&'a self, scope: &mut Scope<'a>) -> f64 {
        let values: Vec<f64> = self
            .sources
            .iter()
            .map(|source| match source {
                Source::Variable(name) => lookup(scope, name),
                Source::Integral(integral) => integral.eval(scope),
                Source::Derivative(derivative) => derivative.eval(scope),
            })
            .collect();
        self.flat.eval(&values).unwrap_or(f64::NAN)
    }

    /// The variables this expression needs bound.
    fn free(&self, out: &mut BTreeSet<String>) {
        for source in &self.sources {
            match source {
                Source::Variable(name) => {
                    out.insert(name.clone());
                }
                Source::Integral(integral) => integral.free(out),
                Source::Derivative(derivative) => {
                    derivative.body.free(out);
                    out.insert(derivative.var.clone());
                }
            }
        }
    }
}

#[derive(Debug)]
struct Integral {
    var: String,
    integrand: Expr,
    /// `None` is the antiderivative, taken from 0 to the value the
    /// surrounding scope gives `var`.
    limits: Option<(Expr, Expr)>,
}

impl Integral {
    fn eval<'a>(&'a self, scope: &mut Scope<'a>) -> f64 {
        let (from, to) = match &self.limits {
            Some((lower, upper)) => (lower.eval(scope), upper.eval(scope)),
            None => (0.0, lookup(scope, &self.var)),
        };
        integrate(
            |t| {
                scope.push((self.var.as_str(), t));
                let value = self.integrand.eval(scope);
                scope.pop();
                value
            },
            from,
            to,
        )
    }

    fn free(&self, out: &mut BTreeSet<String>) {
        let mut inner = BTreeSet::new();
        self.integrand.free(&mut inner);
        inner.remove(&self.var);
        out.extend(inner);
        match &self.limits {
            Some((lower, upper)) => {
                lower.free(out);
                upper.free(out);
            }
            None => {
                out.insert(self.var.clone());
            }
        }
    }
}

/// A derivative exmex could not take symbolically, estimated by central
/// differences.
#[derive(Debug)]
struct Derivative {
    var: String,
    order: u32,
    body: Expr,
}

impl Derivative {
    /// The highest order estimated numerically; beyond it, differences of
    /// quadrature results are mostly noise.
    const MAX_NUMERIC_ORDER: u32 = 2;

    fn eval<'a>(&'a self, scope: &mut Scope<'a>) -> f64 {
        let at = lookup(scope, &self.var);
        if !at.is_finite() {
            return f64::NAN;
        }
        let mut value_at = |t: f64| {
            scope.push((self.var.as_str(), t));
            let value = self.body.eval(scope);
            scope.pop();
            value
        };
        let size = at.abs().max(1.0);
        match self.order {
            1 => {
                let h = 1e-5 * size;
                (value_at(at + h) - value_at(at - h)) / (2.0 * h)
            }
            2 => {
                let h = 1e-3 * size;
                (value_at(at + h) - 2.0 * value_at(at) + value_at(at - h)) / (h * h)
            }
            _ => f64::NAN,
        }
    }
}

/// ∫ₐᵇ f, by adaptive Simpson over a few fixed panels, with a budget on
/// evaluations so a curve that never settles still returns in bounded
/// time. Reversed limits give the negated integral; any non-finite sample
/// makes the result non-finite.
fn integrate(mut f: impl FnMut(f64) -> f64, from: f64, to: f64) -> f64 {
    const PANELS: usize = 8;
    const TOLERANCE: f64 = 1e-12;
    const DEPTH: u32 = 20;
    const BUDGET: usize = 50_000;
    if !(from.is_finite() && to.is_finite()) {
        return f64::NAN;
    }
    if from == to {
        return 0.0;
    }
    let width = (to - from) / PANELS as f64;
    let mut budget = BUDGET;
    let mut total = 0.0;
    for panel in 0..PANELS {
        let a = from + width * panel as f64;
        let b = if panel + 1 == PANELS { to } else { a + width };
        let m = (a + b) / 2.0;
        let (fa, fm, fb) = (f(a), f(m), f(b));
        let whole = simpson(a, b, fa, fm, fb);
        total += refine(
            &mut f,
            [a, b],
            [fa, fm, fb],
            whole,
            TOLERANCE / PANELS as f64,
            DEPTH,
            &mut budget,
        );
    }
    total
}

fn simpson(a: f64, b: f64, fa: f64, fm: f64, fb: f64) -> f64 {
    (b - a) / 6.0 * (fa + 4.0 * fm + fb)
}

fn refine(
    f: &mut impl FnMut(f64) -> f64,
    [a, b]: [f64; 2],
    [fa, fm, fb]: [f64; 3],
    whole: f64,
    tolerance: f64,
    depth: u32,
    budget: &mut usize,
) -> f64 {
    let m = (a + b) / 2.0;
    let (lm, rm) = ((a + m) / 2.0, (m + b) / 2.0);
    let (flm, frm) = (f(lm), f(rm));
    *budget = budget.saturating_sub(2);
    let left = simpson(a, m, fa, flm, fm);
    let right = simpson(m, b, fm, frm, fb);
    let delta = left + right - whole;
    if depth == 0 || *budget == 0 || !delta.is_finite() || delta.abs() <= 15.0 * tolerance {
        return left + right + delta / 15.0;
    }
    refine(
        f,
        [a, m],
        [fa, flm, fm],
        left,
        tolerance / 2.0,
        depth - 1,
        budget,
    ) + refine(
        f,
        [m, b],
        [fm, frm, fb],
        right,
        tolerance / 2.0,
        depth - 1,
        budget,
    )
}

/// A variable in exmex's braced form, which allows any name.
fn variable(name: &str) -> Result<String, EvalError> {
    if name.is_empty() || name.contains(['{', '}', '⟨']) {
        return Err(EvalError::Unsupported(format!("variable name {name:?}")));
    }
    Ok(format!("{{{name}}}"))
}

fn unbrace(name: &str) -> &str {
    name.trim_start_matches('{').trim_end_matches('}')
}

/// The name a variable node stands for.
fn variable_name(node: &MathNode) -> Option<String> {
    match node {
        MathNode::Sym(c) if c.is_alphabetic() => Some(c.to_string()),
        MathNode::Resolved {
            id,
            role: SymbolRole::Variable,
            ..
        } => Some(id.clone()),
        _ => None,
    }
}

/// The left side of `name = …`: a single variable.
fn output_name(list: &[MathNode]) -> Result<String, EvalError> {
    match list {
        [node] if variable_name(node).is_some() => {
            Ok(variable_name(node).expect("checked just above"))
        }
        [] => Err(EvalError::Malformed("nothing before =".to_owned())),
        _ => Err(EvalError::Unsupported(
            "a left side other than one variable".to_owned(),
        )),
    }
}

/// A whole-number exponent, as a derivative's order is written.
fn order(sup: &[MathNode]) -> Option<u32> {
    let digits: String = sup
        .iter()
        .map(|node| match node {
            MathNode::Sym(c) if c.is_ascii_digit() => Some(*c),
            _ => None,
        })
        .collect::<Option<_>>()?;
    digits.parse().ok().filter(|order| *order >= 1)
}

/// `d/dx`, `∂/∂x`, `dⁿ/dxⁿ` and `d(body)/dx`: the variable, the order, and
/// the body when it is written in the numerator. `None` when the fraction
/// is an ordinary one.
fn leibniz<'a>(
    num: &'a [MathNode],
    den: &[MathNode],
) -> Option<(String, u32, Option<&'a [MathNode]>)> {
    let operator = |node: &MathNode| match node {
        MathNode::Sym(c @ ('d' | '∂')) => Some((*c, 1)),
        MathNode::Script {
            base,
            sup: Some(sup),
            sub: None,
        } => match base.as_slice() {
            [MathNode::Sym(c @ ('d' | '∂'))] => Some((*c, order(sup)?)),
            _ => None,
        },
        _ => None,
    };
    let (d, top_order) = operator(num.first()?)?;
    let body = &num[1..];
    let (var, bottom_order) = match den {
        [MathNode::Sym(c), var] if *c == d => (variable_name(var)?, 1),
        [
            MathNode::Sym(c),
            MathNode::Script {
                base,
                sup: Some(sup),
                sub: None,
            },
        ] if *c == d => match base.as_slice() {
            [var] => (variable_name(var)?, order(sup)?),
            _ => return None,
        },
        [
            MathNode::Script {
                base,
                sup: Some(sup),
                sub: None,
            },
        ] => match base.as_slice() {
            [MathNode::Sym(c), var] if *c == d => (variable_name(var)?, order(sup)?),
            _ => return None,
        },
        _ => return None,
    };
    (top_order == bottom_order).then_some((var, top_order, (!body.is_empty()).then_some(body)))
}

/// One reading unit of a list.
#[derive(Clone, Debug, PartialEq)]
enum Piece {
    /// Something with a value: a number, a variable, a bracketed whole.
    Operand(String),
    /// Something that takes the operands after it as its argument.
    Apply(Applier),
    /// `+ - * /`.
    Operator(char),
}

#[derive(Clone, Debug, PartialEq)]
enum Applier {
    Function(&'static str),
    Derivative { var: String, order: u32 },
}

/// Turns trees into exmex text, collecting the integrals and numeric
/// derivatives that text refers to by placeholder.
#[derive(Default)]
struct Translator {
    placeholders: Vec<(String, Source)>,
}

impl Translator {
    /// `list` in exmex syntax: every multiplication explicit, every
    /// variable braced so no name collides with an exmex operator (`e`,
    /// `E`, `PI`).
    fn translate(&mut self, list: &[MathNode]) -> Result<String, EvalError> {
        let pieces = self.pieces(list)?;
        self.join(&pieces)
    }

    /// Parses text this translator produced, resolving its placeholders.
    fn compile(&self, text: &str) -> Result<Expr, EvalError> {
        let flat =
            exmex::parse::<f64>(text).map_err(|error| EvalError::Malformed(error.to_string()))?;
        let sources = flat
            .var_names()
            .iter()
            .map(|braced| {
                let name = unbrace(braced);
                self.source(name)
                    .cloned()
                    .unwrap_or_else(|| Source::Variable(name.to_owned()))
            })
            .collect();
        Ok(Expr { flat, sources })
    }

    fn source(&self, name: &str) -> Option<&Source> {
        self.placeholders
            .iter()
            .find(|(placeholder, _)| placeholder == name)
            .map(|(_, source)| source)
    }

    fn placeholder(&mut self, source: Source) -> String {
        let name = format!("⟨{}⟩", self.placeholders.len());
        let text = format!("{{{name}}}");
        self.placeholders.push((name, source));
        text
    }

    fn pieces(&mut self, list: &[MathNode]) -> Result<Vec<Piece>, EvalError> {
        let mut out = Vec::new();
        let mut index = 0;
        while index < list.len() {
            let node = &list[index];
            index += 1;
            let piece = match node {
                MathNode::Sym(c) if c.is_ascii_digit() || *c == '.' => {
                    let mut number = c.to_string();
                    while let Some(MathNode::Sym(next)) = list.get(index)
                        && (next.is_ascii_digit() || *next == '.')
                    {
                        number.push(*next);
                        index += 1;
                    }
                    if number.parse::<f64>().is_err() {
                        return Err(EvalError::Malformed(format!("number {number}")));
                    }
                    Piece::Operand(number)
                }
                MathNode::Sym(c) if c.is_whitespace() => continue,
                MathNode::Sym('π') => Piece::Operand("π".to_owned()),
                MathNode::Sym('τ') => Piece::Operand("τ".to_owned()),
                MathNode::Sym(c) if c.is_alphabetic() => Piece::Operand(variable(&c.to_string())?),
                MathNode::Sym('+') => Piece::Operator('+'),
                MathNode::Sym('-' | '−') => Piece::Operator('-'),
                MathNode::Sym('*' | '×' | '·' | '∗') => Piece::Operator('*'),
                MathNode::Sym('/' | '÷') => Piece::Operator('/'),
                MathNode::Sym(c) => {
                    return Err(EvalError::Unsupported(format!("the symbol {c}")));
                }
                MathNode::Resolved { id, role, .. } => resolved(id, *role)?,
                MathNode::Frac { num, den } => match leibniz(num, den) {
                    Some((var, order, None)) => Piece::Apply(Applier::Derivative { var, order }),
                    Some((var, order, Some(body))) => {
                        let body = self.translate(body)?;
                        Piece::Operand(self.derive(&body, &var, order)?)
                    }
                    None => Piece::Operand(format!(
                        "(({})/({}))",
                        self.translate(num)?,
                        self.translate(den)?
                    )),
                },
                MathNode::Script { base, sup, sub } => {
                    Piece::Operand(self.script(base, sup.as_deref(), sub.as_deref())?)
                }
                MathNode::Group { open, close, body } => Piece::Operand(match (open, close) {
                    ('(', ')') | ('[', ']') => format!("({})", self.translate(body)?),
                    ('|', '|') => format!("abs({})", self.translate(body)?),
                    _ => {
                        return Err(EvalError::Unsupported(format!(
                            "the brackets {open}{close}"
                        )));
                    }
                }),
                MathNode::Sqrt { body } => {
                    Piece::Operand(format!("sqrt({})", self.translate(body)?))
                }
                MathNode::Accent { kind, .. } => {
                    return Err(EvalError::Unsupported(format!(
                        "the {} accent",
                        kind.keyword()
                    )));
                }
                MathNode::BigOp {
                    kind: BigOp::Integral,
                    lower,
                    upper,
                } => {
                    let (piece, used) = self.integral(lower, upper, &list[index..])?;
                    index += used;
                    piece
                }
                MathNode::BigOp { kind, .. } => {
                    return Err(EvalError::Unsupported(format!(
                        "the {} operator",
                        kind.keyword()
                    )));
                }
            };
            out.push(piece);
        }
        Ok(out)
    }

    /// An integral whose integrand and differential are the start of
    /// `rest`; returns its placeholder and how much of `rest` it took.
    fn integral(
        &mut self,
        lower: &[MathNode],
        upper: &[MathNode],
        rest: &[MathNode],
    ) -> Result<(Piece, usize), EvalError> {
        // The differential closing this integral is the first one not
        // claimed by an integral nested inside it.
        let mut open = 0;
        let mut closing = None;
        for (at, node) in rest.iter().enumerate() {
            match node {
                MathNode::BigOp {
                    kind: BigOp::Integral,
                    ..
                } => open += 1,
                MathNode::Sym('d') => {
                    if let Some(var) = rest.get(at + 1).and_then(variable_name) {
                        if open == 0 {
                            closing = Some((at, var));
                            break;
                        }
                        open -= 1;
                    }
                }
                _ => {}
            }
        }
        let Some((at, var)) = closing else {
            return Err(EvalError::Malformed(
                "an integral without its differential, like dx".to_owned(),
            ));
        };

        // `∫ dx` integrates 1.
        let integrand = match &rest[..at] {
            [] => "1".to_owned(),
            body => self.translate(body)?,
        };
        let integrand = self.compile(&integrand)?;
        let limits = match (lower.is_empty(), upper.is_empty()) {
            (true, true) => None,
            (false, false) => {
                let lower = self.translate(lower)?;
                let upper = self.translate(upper)?;
                Some((self.compile(&lower)?, self.compile(&upper)?))
            }
            _ => {
                return Err(EvalError::Malformed(
                    "an integral with only one limit".to_owned(),
                ));
            }
        };
        let placeholder = self.placeholder(Source::Integral(Rc::new(Integral {
            var,
            integrand,
            limits,
        })));
        Ok((Piece::Operand(placeholder), at + 2))
    }

    /// The `order`-th derivative of `body` (exmex text) in `var`, as exmex
    /// text: symbolic where exmex can take it, numeric otherwise.
    fn derive(&mut self, body: &str, var: &str, order: u32) -> Result<String, EvalError> {
        let flat =
            exmex::parse::<f64>(body).map_err(|error| EvalError::Malformed(error.to_string()))?;
        let names: Vec<&str> = flat.var_names().iter().map(|name| unbrace(name)).collect();
        if !names.iter().any(|name| self.source(name).is_some()) {
            let Some(index) = names.iter().position(|name| *name == var) else {
                // Nothing in the body varies with `var`.
                return Ok("0".to_owned());
            };
            if let Ok(derived) = flat.partial_nth(index, order as usize) {
                return Ok(format!("({})", derived.unparse()));
            }
        }
        if order > Derivative::MAX_NUMERIC_ORDER {
            return Err(EvalError::Unsupported(format!(
                "a derivative of order {order} that has to be estimated numerically"
            )));
        }
        let body = self.compile(body)?;
        Ok(self.placeholder(Source::Derivative(Rc::new(Derivative {
            var: var.to_owned(),
            order,
            body,
        }))))
    }

    fn script(
        &mut self,
        base: &[MathNode],
        sup: Option<&[MathNode]>,
        sub: Option<&[MathNode]>,
    ) -> Result<String, EvalError> {
        let base = match sub {
            None => {
                let pieces = self.pieces(base)?;
                if let [Piece::Apply(_)] = pieces.as_slice() {
                    return Err(EvalError::Unsupported(
                        "powers of a function, like sin²x".to_owned(),
                    ));
                }
                format!("({})", self.join(&pieces)?)
            }
            // An index is part of a variable's name: x₁ is its own variable.
            Some(sub) => {
                let [node] = base else {
                    return Err(EvalError::Unsupported(
                        "an index on anything but a variable".to_owned(),
                    ));
                };
                let Some(name) = variable_name(node) else {
                    return Err(EvalError::Unsupported(
                        "an index on anything but a variable".to_owned(),
                    ));
                };
                let index: Option<String> = sub
                    .iter()
                    .map(|node| match node {
                        MathNode::Sym(c) if c.is_alphanumeric() => Some(*c),
                        _ => None,
                    })
                    .collect();
                let Some(index) = index.filter(|index| !index.is_empty()) else {
                    return Err(EvalError::Unsupported(
                        "an index other than letters and digits".to_owned(),
                    ));
                };
                variable(&format!("{name}_{index}"))?
            }
        };
        match sup {
            Some(sup) => Ok(format!("({base})^({})", self.translate(sup)?)),
            None => Ok(base),
        }
    }

    /// `applier` applied to the argument starting at `pieces[index]`: the
    /// run of operands there, or — when another applier comes first — that
    /// application, so `d/dx sin x` is `d/dx (sin x)`. Returns the text and
    /// the index after the argument.
    fn apply(
        &mut self,
        applier: &Applier,
        pieces: &[Piece],
        mut index: usize,
    ) -> Result<(String, usize), EvalError> {
        let argument = if let Some(Piece::Apply(inner)) = pieces.get(index) {
            let (text, next) = self.apply(inner, pieces, index + 1)?;
            index = next;
            text
        } else {
            let run: Vec<&str> = pieces[index..]
                .iter()
                .map_while(|piece| match piece {
                    Piece::Operand(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            if run.is_empty() {
                let name = match applier {
                    Applier::Function(name) => (*name).to_owned(),
                    Applier::Derivative { var, .. } => format!("d/d{var}"),
                };
                return Err(EvalError::Malformed(format!("{name} with no argument")));
            }
            index += run.len();
            run.join("*")
        };
        let applied = match applier {
            Applier::Function(name) => format!("{name}({argument})"),
            Applier::Derivative { var, order } => self.derive(&argument, var, *order)?,
        };
        Ok((applied, index))
    }

    /// Pieces to text. Juxtaposed operands multiply; a function or
    /// derivative takes the run of juxtaposed operands after it, stopping
    /// at an operator or the next applier, so `sin x cos x` is
    /// `sin(x)·cos(x)`.
    fn join(&mut self, pieces: &[Piece]) -> Result<String, EvalError> {
        let mut out = String::new();
        // Whether the text so far ends in a value, so a following value is
        // a product.
        let mut after_value = false;
        let mut index = 0;
        while index < pieces.len() {
            match &pieces[index] {
                Piece::Operator(op) => {
                    out.push(*op);
                    after_value = false;
                    index += 1;
                }
                Piece::Operand(text) => {
                    if after_value {
                        out.push('*');
                    }
                    out.push_str(text);
                    after_value = true;
                    index += 1;
                }
                Piece::Apply(applier) => {
                    let (applied, next) = self.apply(applier, pieces, index + 1)?;
                    index = next;
                    if after_value {
                        out.push('*');
                    }
                    out.push_str(&applied);
                    after_value = true;
                }
            }
        }
        if out.is_empty() {
            return Err(EvalError::Malformed("an empty expression".to_owned()));
        }
        if matches!(pieces.last(), Some(Piece::Operator(_))) {
            return Err(EvalError::Malformed(format!("{out} ends in an operator")));
        }
        Ok(out)
    }
}

fn resolved(id: &str, role: SymbolRole) -> Result<Piece, EvalError> {
    Ok(match role {
        SymbolRole::Variable => Piece::Operand(variable(id)?),
        SymbolRole::Constant => Piece::Operand(
            match id {
                "pi" => "π",
                "tau" => "τ",
                "euler_number" => "E",
                "golden_ratio" => "1.618033988749895",
                _ => return Err(EvalError::Unsupported(format!("the constant {id}"))),
            }
            .to_owned(),
        ),
        SymbolRole::Function => Piece::Apply(Applier::Function(match id {
            "sin" => "sin",
            "cos" => "cos",
            "tan" => "tan",
            "exp" => "exp",
            // Natural, as in analysis texts; exmex reads `log` the same way.
            "ln" | "log" => "ln",
            _ => {
                return Err(EvalError::Unsupported(format!(
                    "the function {id}, which has no definition"
                )));
            }
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::math_notation::parse;

    const SIN: &str = "sym{function|sin|plain}{sin}";
    const COS: &str = "sym{function|cos|plain}{cos}";

    fn curve(notation: &str) -> Curve {
        Curve::parse(&parse(notation)).unwrap_or_else(|error| panic!("{notation}: {error}"))
    }

    fn translate(notation: &str) -> Result<String, EvalError> {
        Translator::default().translate(&parse(notation))
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// For results that pass through quadrature or finite differences.
    fn near(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() < tolerance
    }

    #[test]
    fn a_cubic_reads_as_a_curve_in_x() {
        let cubic = curve("y=x^3");
        assert_eq!(cubic.output.as_deref(), Some("y"));
        assert_eq!(cubic.input.as_deref(), Some("x"));
        assert!(close(cubic.eval(2.0), 8.0));
        assert!(close(cubic.eval(-1.5), -3.375));
    }

    #[test]
    fn juxtaposition_multiplies() {
        assert!(close(curve("2x").eval(3.0), 6.0));
        assert!(close(curve("y=3(x+1)").eval(1.0), 6.0));
        assert!(close(curve("y=(x+1)(x-1)").eval(3.0), 8.0));
        // Two letters are two variables, not a name.
        assert_eq!(translate("xy").as_deref(), Ok("{x}*{y}"));
    }

    #[test]
    fn fractions_roots_and_bars_nest() {
        assert!(close(curve("y=1/{x+1}").eval(1.0), 0.5));
        assert!(close(curve("y=sqrt{x}").eval(9.0), 3.0));
        assert!(close(curve("y=|x-5|").eval(2.0), 3.0));
        assert!(close(curve("y=x^{1/2}").eval(16.0), 4.0));
    }

    #[test]
    fn letters_are_variables_even_where_exmex_has_a_constant() {
        // A plain `e` is whatever the reader typed, not Euler's number.
        let plain = curve("y=2e");
        assert_eq!(plain.input.as_deref(), Some("e"));
        assert!(close(plain.eval(1.0), 2.0));
        let euler = curve("y=sym{constant|euler_number|plain}{e}");
        assert_eq!(euler.input, None);
        assert!(close(euler.eval(0.0), std::f64::consts::E));
    }

    #[test]
    fn resolved_functions_take_the_following_product() {
        let wave = curve(&format!("y={SIN}2x"));
        assert!(close(wave.eval(0.25), 0.5f64.sin()));
        let product = curve(&format!("y={SIN}x {COS}x"));
        assert!(close(product.eval(0.7), 0.7f64.sin() * 0.7f64.cos()));
        let pi = curve(&format!("y={COS}(sym{{constant|pi|plain}}{{π}}x)"));
        assert!(close(pi.eval(1.0), -1.0));
    }

    #[test]
    fn indices_name_variables() {
        let indexed = curve("y=x_1^2");
        assert_eq!(indexed.input.as_deref(), Some("x_1"));
        assert!(close(indexed.eval(3.0), 9.0));
    }

    #[test]
    fn undefined_points_are_nan() {
        assert!(curve("y=1/x").eval(0.0).is_infinite());
        assert!(curve("y=sqrt{x}").eval(-1.0).is_nan());
    }

    #[test]
    fn leibniz_fractions_differentiate_symbolically() {
        // Applied to what follows, bracketed or not.
        assert!(close(curve("y=d/{dx}x^2").eval(3.0), 6.0));
        assert!(close(curve("y=d/{dx}(x^3-2x)").eval(2.0), 10.0));
        assert!(close(
            curve(&format!("y=d/{{dx}}{SIN}x")).eval(0.4),
            0.4f64.cos()
        ));
        // The body written in the numerator.
        assert!(close(curve("y={d(x^2)}/{dx}").eval(3.0), 6.0));
        // Partial and higher-order forms.
        assert!(close(curve("y=∂/{∂x}x").eval(5.0), 1.0));
        assert!(close(curve("y={d^2}/{dx^2}x^3").eval(2.0), 12.0));
        assert!(close(curve("y=2d/{dx}x^2+1").eval(1.0), 5.0));
        // Nothing varies with x: zero, and no input left.
        let flat = curve("y=d/{dx}5");
        assert_eq!(flat.input, None);
        assert!(close(flat.eval(0.0), 0.0));
        // Symbolic, so no placeholder remains.
        assert!(!curve("y=d/{dx}x^2").expression().contains('⟨'));
    }

    #[test]
    fn a_d_over_something_else_is_an_ordinary_fraction() {
        let ratio = curve("y=d/2");
        assert_eq!(ratio.input.as_deref(), Some("d"));
        assert!(close(ratio.eval(3.0), 1.5));
    }

    #[test]
    fn definite_integrals_take_their_limits_from_the_curve() {
        let area = curve("y=int{0}{x}t^2dt");
        assert_eq!(area.input.as_deref(), Some("x"));
        assert!(near(area.eval(3.0), 9.0, 1e-9));
        assert!(near(area.eval(-3.0), -9.0, 1e-9));
        // `∫ dt` integrates 1.
        assert!(near(curve("y=int{0}{x}dt").eval(2.5), 2.5, 1e-9));
        // The integration variable shadows the curve's own.
        assert!(near(curve("y=int{0}{x}xdx").eval(2.0), 2.0, 1e-9));
        // The integrand may use the curve's variable too.
        assert!(near(curve("y=int{0}{1}xtdt").eval(2.0), 1.0, 1e-9));
        // Oscillating integrands still converge.
        let wave = curve(&format!("y=int{{0}}{{x}}{SIN}tdt"));
        assert!(near(wave.eval(2.0), 1.0 - 2.0f64.cos(), 1e-9));
    }

    #[test]
    fn constant_integrals_and_their_surroundings() {
        let half = curve("y=int{0}{1}xdx");
        assert_eq!(half.input, None);
        assert!(near(half.eval(0.0), 0.5, 1e-12));
        // The differential closes the integral; what follows is outside it.
        assert!(near(curve("y=2int{0}{1}xdx+1").eval(0.0), 2.0, 1e-12));
        assert!(near(curve("y=int{0}{1}xdxx").eval(4.0), 2.0, 1e-12));
    }

    #[test]
    fn an_indefinite_integral_is_the_antiderivative_through_zero() {
        let primitive = curve("y=int{}{}xdx");
        assert_eq!(primitive.input.as_deref(), Some("x"));
        assert!(near(primitive.eval(2.0), 2.0, 1e-9));
        assert!(near(primitive.eval(-4.0), 8.0, 1e-9));
    }

    #[test]
    fn nested_integrals_close_their_own_differentials_first() {
        // ∫₀ˣ ∫₀ᵗ s ds dt = x³/6
        let twice = curve("y=int{0}{x}int{0}{t}sdsdt");
        assert!(near(twice.eval(3.0), 4.5, 1e-9));
    }

    #[test]
    fn calculus_composes_both_ways() {
        // Symbolic inside an integrand.
        let back = curve("y=int{0}{x}d/{dt}(t^3)dt");
        assert!(near(back.eval(2.0), 8.0, 1e-9));
        // Numeric around an integral: the fundamental theorem.
        let ftc = curve(&format!("y=d/{{dx}}int{{0}}{{x}}{COS}(t^2)dt"));
        assert!(ftc.expression().contains('⟨'));
        assert!(near(ftc.eval(1.3), (1.3f64 * 1.3).cos(), 1e-6));
        let second = curve("y={d^2}/{dx^2}int{0}{x}t^3dt");
        assert!(near(second.eval(2.0), 12.0, 1e-3));
    }

    #[test]
    fn what_has_no_numeric_reading_says_so() {
        for (notation, expected) in [
            ("y=sum{n=1}{5}x", "the sum operator"),
            ("y=oint{}{}xdx", "the oint operator"),
            ("y=vec{x}", "the vec accent"),
            ("y=sym{function|f|plain}{f}(x)", "the function f"),
            ("y=x!", "the symbol !"),
            ("y=x=2", "more than one ="),
            ("xy", "more than one variable"),
            ("y=", "an empty expression"),
            ("y=x+", "ends in an operator"),
            ("y=sym{function|sin|plain}{sin}", "sin with no argument"),
            ("y=d/{dx}", "d/dx with no argument"),
            ("y=int{0}{x}t", "without its differential"),
            ("y=int{0}{}tdt", "only one limit"),
            ("y={d^3}/{dx^3}int{0}{x}tdt", "order 3"),
        ] {
            let error = Curve::parse(&parse(notation))
                .expect_err(notation)
                .to_string();
            assert!(error.contains(expected), "{notation}: {error}");
        }
    }
}
