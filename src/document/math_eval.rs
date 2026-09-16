//! Numeric meaning for a math tree, for plotting. The tree stays
//! presentational (MATH.md); this module reads one tree as an expression,
//! translates it to exmex syntax and lets exmex parse and evaluate it.
//!
//! The reading is deliberately literal. Every plain letter is a variable of
//! its own (`xy` is `x·y`, as in TeX), juxtaposition multiplies, and only
//! symbols resolved as functions or constants mean one. Anything without a
//! numeric reading yet — integrals, accents, user functions — is an
//! [`EvalError::Unsupported`] naming what it met, never a guess.

use std::fmt;

use exmex::prelude::*;

use super::math::{MathList, MathNode, SymbolRole};

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
    expression: FlatEx<f64>,
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
        let expression = exmex::parse::<f64>(&translate(body)?)
            .map_err(|error| EvalError::Malformed(error.to_string()))?;
        let input = match expression.var_names() {
            [] => None,
            [name] => Some(unbrace(name).to_owned()),
            names => {
                return Err(EvalError::Unsupported(format!(
                    "a curve in more than one variable ({})",
                    names
                        .iter()
                        .map(|n| unbrace(n))
                        .collect::<Vec<_>>()
                        .join(", ")
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
        let inputs: &[f64] = if self.input.is_some() { &[x] } else { &[] };
        self.expression.eval(inputs).unwrap_or(f64::NAN)
    }

    /// The exmex form of the expression, for diagnostics.
    pub fn expression(&self) -> &str {
        self.expression.unparse()
    }
}

/// `list` in exmex syntax: every multiplication explicit, every variable
/// braced so no name collides with an exmex operator (`e`, `E`, `PI`).
pub fn translate(list: &[MathNode]) -> Result<String, EvalError> {
    join(&pieces(list)?)
}

/// A variable in exmex's braced form, which allows any name.
fn variable(name: &str) -> Result<String, EvalError> {
    if name.is_empty() || name.contains(['{', '}']) {
        return Err(EvalError::Unsupported(format!("variable name {name:?}")));
    }
    Ok(format!("{{{name}}}"))
}

fn unbrace(name: &str) -> &str {
    name.trim_start_matches('{').trim_end_matches('}')
}

/// The left side of `name = …`: a single variable.
fn output_name(list: &[MathNode]) -> Result<String, EvalError> {
    match list {
        [MathNode::Sym(c)] if c.is_alphabetic() => Ok(c.to_string()),
        [
            MathNode::Resolved {
                id,
                role: SymbolRole::Variable,
                ..
            },
        ] => Ok(id.clone()),
        [] => Err(EvalError::Malformed("nothing before =".to_owned())),
        _ => Err(EvalError::Unsupported(
            "a left side other than one variable".to_owned(),
        )),
    }
}

/// One reading unit of a list.
#[derive(Clone, Debug, PartialEq)]
enum Piece {
    /// Something with a value: a number, a variable, a bracketed whole.
    Operand(String),
    /// A function waiting for its argument.
    Function(&'static str),
    /// `+ - * /`.
    Operator(char),
}

fn pieces(list: &[MathNode]) -> Result<Vec<Piece>, EvalError> {
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
            MathNode::Sym(c) => return Err(EvalError::Unsupported(format!("the symbol {c}"))),
            MathNode::Resolved { id, role, .. } => resolved(id, *role)?,
            MathNode::Frac { num, den } => {
                Piece::Operand(format!("(({})/({}))", translate(num)?, translate(den)?))
            }
            MathNode::Script { base, sup, sub } => Piece::Operand(script(base, sup, sub)?),
            MathNode::Group { open, close, body } => Piece::Operand(match (open, close) {
                ('(', ')') | ('[', ']') => format!("({})", translate(body)?),
                ('|', '|') => format!("abs({})", translate(body)?),
                _ => {
                    return Err(EvalError::Unsupported(format!(
                        "the brackets {open}{close}"
                    )));
                }
            }),
            MathNode::Sqrt { body } => Piece::Operand(format!("sqrt({})", translate(body)?)),
            MathNode::Accent { kind, .. } => {
                return Err(EvalError::Unsupported(format!(
                    "the {} accent",
                    kind.keyword()
                )));
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
        SymbolRole::Function => Piece::Function(match id {
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
        }),
    })
}

fn script(
    base: &MathList,
    sup: &Option<MathList>,
    sub: &Option<MathList>,
) -> Result<String, EvalError> {
    let base = match sub {
        None => match pieces(base)?.as_slice() {
            [Piece::Function(_)] => {
                return Err(EvalError::Unsupported(
                    "powers of a function, like sin²x".to_owned(),
                ));
            }
            _ => format!("({})", translate(base)?),
        },
        // An index is part of a variable's name: x₁ is its own variable.
        Some(sub) => {
            let name = match base.as_slice() {
                [MathNode::Sym(c)] if c.is_alphabetic() => c.to_string(),
                [
                    MathNode::Resolved {
                        id,
                        role: SymbolRole::Variable,
                        ..
                    },
                ] => id.clone(),
                _ => {
                    return Err(EvalError::Unsupported(
                        "an index on anything but a variable".to_owned(),
                    ));
                }
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
        Some(sup) => Ok(format!("({base})^({})", translate(sup)?)),
        None => Ok(base),
    }
}

/// Pieces to text. Juxtaposed operands multiply; a function takes the run
/// of juxtaposed operands after it, stopping at an operator or the next
/// function, so `sin x cos x` is `sin(x)·cos(x)`.
fn join(pieces: &[Piece]) -> Result<String, EvalError> {
    let mut out = String::new();
    // Whether the text so far ends in a value, so a following value is a
    // product.
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
            Piece::Function(name) => {
                index += 1;
                let arguments: Vec<&str> = pieces[index..]
                    .iter()
                    .map_while(|piece| match piece {
                        Piece::Operand(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                if arguments.is_empty() {
                    return Err(EvalError::Malformed(format!("{name} with no argument")));
                }
                index += arguments.len();
                if after_value {
                    out.push('*');
                }
                out.push_str(&format!("{name}({})", arguments.join("*")));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::math_notation::parse;

    fn curve(notation: &str) -> Curve {
        Curve::parse(&parse(notation)).unwrap_or_else(|error| panic!("{notation}: {error}"))
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
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
        assert_eq!(translate(&parse("xy")).as_deref(), Ok("{x}*{y}"));
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
        let wave = curve("y=sym{function|sin|plain}{sin}2x");
        assert!(close(wave.eval(0.25), 0.5f64.sin()));
        let product = curve("y=sym{function|sin|plain}{sin}x sym{function|cos|plain}{cos}x");
        assert!(close(product.eval(0.7), 0.7f64.sin() * 0.7f64.cos()));
        let pi = curve("y=sym{function|cos|plain}{cos}(sym{constant|pi|plain}{π}x)");
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
    fn what_has_no_numeric_reading_says_so() {
        for (notation, expected) in [
            ("y=int{0}{1}x", "the int operator"),
            ("y=vec{x}", "the vec accent"),
            ("y=sym{function|f|plain}{f}(x)", "the function f"),
            ("y=x!", "the symbol !"),
            ("y=x=2", "more than one ="),
            ("xy", "more than one variable"),
            ("y=", "an empty expression"),
            ("y=x+", "ends in an operator"),
            ("y=sym{function|sin|plain}{sin}", "sin with no argument"),
        ] {
            let error = Curve::parse(&parse(notation))
                .expect_err(notation)
                .to_string();
            assert!(error.contains(expected), "{notation}: {error}");
        }
    }
}
