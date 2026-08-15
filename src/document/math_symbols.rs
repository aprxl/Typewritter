//! The set of things a reader can reach by name inside an expression, which
//! is what makes notation typable without a symbol picker. Names are the ones
//! a LaTeX-trained reader already has in their fingers, because that is the
//! vocabulary this reader arrives with.

use super::math::SymbolRole;

/// One completion offered inside an expression.
pub struct Symbol {
    /// What you type to reach it.
    pub name: &'static str,
    /// What it inserts.
    pub glyph: char,
    /// The heading it is listed under.
    pub group: &'static str,
}

impl Symbol {
    /// Only alphabetic catalog entries have persistent semantic identity.
    pub fn role(&self) -> Option<SymbolRole> {
        matches!(self.group, "Variable" | "Greek").then_some(SymbolRole::Variable)
    }
}

/// One visual spelling of an identified alphabetic symbol.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Variant {
    pub key: &'static str,
    pub name: &'static str,
    pub glyph: char,
}

macro_rules! variable {
    ($name:literal, $glyph:literal) => {
        Symbol {
            name: $name,
            glyph: $glyph,
            group: "Variable",
        }
    };
}

pub const SYMBOLS: &[Symbol] = &[
    variable!("a", 'a'),
    variable!("b", 'b'),
    variable!("c", 'c'),
    variable!("d", 'd'),
    variable!("e", 'e'),
    variable!("f", 'f'),
    variable!("g", 'g'),
    variable!("h", 'h'),
    variable!("i", 'i'),
    variable!("j", 'j'),
    variable!("k", 'k'),
    variable!("l", 'l'),
    variable!("m", 'm'),
    variable!("n", 'n'),
    variable!("o", 'o'),
    variable!("p", 'p'),
    variable!("q", 'q'),
    variable!("r", 'r'),
    variable!("s", 's'),
    variable!("t", 't'),
    variable!("u", 'u'),
    variable!("v", 'v'),
    variable!("w", 'w'),
    variable!("x", 'x'),
    variable!("y", 'y'),
    variable!("z", 'z'),
    variable!("A", 'A'),
    variable!("B", 'B'),
    variable!("C", 'C'),
    variable!("D", 'D'),
    variable!("E", 'E'),
    variable!("F", 'F'),
    variable!("G", 'G'),
    variable!("H", 'H'),
    variable!("I", 'I'),
    variable!("J", 'J'),
    variable!("K", 'K'),
    variable!("L", 'L'),
    variable!("M", 'M'),
    variable!("N", 'N'),
    variable!("O", 'O'),
    variable!("P", 'P'),
    variable!("Q", 'Q'),
    variable!("R", 'R'),
    variable!("S", 'S'),
    variable!("T", 'T'),
    variable!("U", 'U'),
    variable!("V", 'V'),
    variable!("W", 'W'),
    variable!("X", 'X'),
    variable!("Y", 'Y'),
    variable!("Z", 'Z'),
    Symbol {
        name: "alpha",
        glyph: 'α',
        group: "Greek",
    },
    Symbol {
        name: "beta",
        glyph: 'β',
        group: "Greek",
    },
    Symbol {
        name: "gamma",
        glyph: 'γ',
        group: "Greek",
    },
    Symbol {
        name: "delta",
        glyph: 'δ',
        group: "Greek",
    },
    Symbol {
        name: "epsilon",
        glyph: 'ε',
        group: "Greek",
    },
    Symbol {
        name: "zeta",
        glyph: 'ζ',
        group: "Greek",
    },
    Symbol {
        name: "eta",
        glyph: 'η',
        group: "Greek",
    },
    Symbol {
        name: "theta",
        glyph: 'θ',
        group: "Greek",
    },
    Symbol {
        name: "iota",
        glyph: 'ι',
        group: "Greek",
    },
    Symbol {
        name: "kappa",
        glyph: 'κ',
        group: "Greek",
    },
    Symbol {
        name: "lambda",
        glyph: 'λ',
        group: "Greek",
    },
    Symbol {
        name: "mu",
        glyph: 'μ',
        group: "Greek",
    },
    Symbol {
        name: "nu",
        glyph: 'ν',
        group: "Greek",
    },
    Symbol {
        name: "xi",
        glyph: 'ξ',
        group: "Greek",
    },
    Symbol {
        name: "omicron",
        glyph: 'ο',
        group: "Greek",
    },
    Symbol {
        name: "pi",
        glyph: 'π',
        group: "Greek",
    },
    Symbol {
        name: "rho",
        glyph: 'ρ',
        group: "Greek",
    },
    Symbol {
        name: "sigma",
        glyph: 'σ',
        group: "Greek",
    },
    Symbol {
        name: "tau",
        glyph: 'τ',
        group: "Greek",
    },
    Symbol {
        name: "upsilon",
        glyph: 'υ',
        group: "Greek",
    },
    Symbol {
        name: "phi",
        glyph: 'φ',
        group: "Greek",
    },
    Symbol {
        name: "chi",
        glyph: 'χ',
        group: "Greek",
    },
    Symbol {
        name: "psi",
        glyph: 'ψ',
        group: "Greek",
    },
    Symbol {
        name: "omega",
        glyph: 'ω',
        group: "Greek",
    },
    Symbol {
        name: "Gamma",
        glyph: 'Γ',
        group: "Greek",
    },
    Symbol {
        name: "Delta",
        glyph: 'Δ',
        group: "Greek",
    },
    Symbol {
        name: "Theta",
        glyph: 'Θ',
        group: "Greek",
    },
    Symbol {
        name: "Lambda",
        glyph: 'Λ',
        group: "Greek",
    },
    Symbol {
        name: "Xi",
        glyph: 'Ξ',
        group: "Greek",
    },
    Symbol {
        name: "Pi",
        glyph: 'Π',
        group: "Greek",
    },
    Symbol {
        name: "Sigma",
        glyph: 'Σ',
        group: "Greek",
    },
    Symbol {
        name: "Phi",
        glyph: 'Φ',
        group: "Greek",
    },
    Symbol {
        name: "Psi",
        glyph: 'Ψ',
        group: "Greek",
    },
    Symbol {
        name: "Omega",
        glyph: 'Ω',
        group: "Greek",
    },
    Symbol {
        name: "leq",
        glyph: '≤',
        group: "Relations",
    },
    Symbol {
        name: "geq",
        glyph: '≥',
        group: "Relations",
    },
    Symbol {
        name: "neq",
        glyph: '≠',
        group: "Relations",
    },
    Symbol {
        name: "approx",
        glyph: '≈',
        group: "Relations",
    },
    Symbol {
        name: "equiv",
        glyph: '≡',
        group: "Relations",
    },
    Symbol {
        name: "sim",
        glyph: '∼',
        group: "Relations",
    },
    Symbol {
        name: "propto",
        glyph: '∝',
        group: "Relations",
    },
    Symbol {
        name: "ll",
        glyph: '≪',
        group: "Relations",
    },
    Symbol {
        name: "gg",
        glyph: '≫',
        group: "Relations",
    },
    Symbol {
        name: "times",
        glyph: '×',
        group: "Operators",
    },
    Symbol {
        name: "cdot",
        glyph: '·',
        group: "Operators",
    },
    Symbol {
        name: "div",
        glyph: '÷',
        group: "Operators",
    },
    Symbol {
        name: "pm",
        glyph: '±',
        group: "Operators",
    },
    Symbol {
        name: "mp",
        glyph: '∓',
        group: "Operators",
    },
    Symbol {
        name: "ast",
        glyph: '∗',
        group: "Operators",
    },
    Symbol {
        name: "star",
        glyph: '⋆',
        group: "Operators",
    },
    Symbol {
        name: "to",
        glyph: '→',
        group: "Arrows",
    },
    Symbol {
        name: "gets",
        glyph: '←',
        group: "Arrows",
    },
    Symbol {
        name: "rightarrow",
        glyph: '→',
        group: "Arrows",
    },
    Symbol {
        name: "leftarrow",
        glyph: '←',
        group: "Arrows",
    },
    Symbol {
        name: "Rightarrow",
        glyph: '⇒',
        group: "Arrows",
    },
    Symbol {
        name: "Leftarrow",
        glyph: '⇐',
        group: "Arrows",
    },
    Symbol {
        name: "leftrightarrow",
        glyph: '↔',
        group: "Arrows",
    },
    Symbol {
        name: "mapsto",
        glyph: '↦',
        group: "Arrows",
    },
    Symbol {
        name: "in",
        glyph: '∈',
        group: "Sets",
    },
    Symbol {
        name: "notin",
        glyph: '∉',
        group: "Sets",
    },
    Symbol {
        name: "subset",
        glyph: '⊂',
        group: "Sets",
    },
    Symbol {
        name: "subseteq",
        glyph: '⊆',
        group: "Sets",
    },
    Symbol {
        name: "cup",
        glyph: '∪',
        group: "Sets",
    },
    Symbol {
        name: "cap",
        glyph: '∩',
        group: "Sets",
    },
    Symbol {
        name: "emptyset",
        glyph: '∅',
        group: "Sets",
    },
    Symbol {
        name: "forall",
        glyph: '∀',
        group: "Sets",
    },
    Symbol {
        name: "exists",
        glyph: '∃',
        group: "Sets",
    },
    Symbol {
        name: "neg",
        glyph: '¬',
        group: "Sets",
    },
    Symbol {
        name: "land",
        glyph: '∧',
        group: "Sets",
    },
    Symbol {
        name: "lor",
        glyph: '∨',
        group: "Sets",
    },
    Symbol {
        name: "infty",
        glyph: '∞',
        group: "Calculus",
    },
    Symbol {
        name: "partial",
        glyph: '∂',
        group: "Calculus",
    },
    Symbol {
        name: "nabla",
        glyph: '∇',
        group: "Calculus",
    },
    Symbol {
        name: "deg",
        glyph: '°',
        group: "Calculus",
    },
    Symbol {
        name: "prime",
        glyph: '′',
        group: "Calculus",
    },
    Symbol {
        name: "dots",
        glyph: '…',
        group: "Calculus",
    },
    Symbol {
        name: "cdots",
        glyph: '⋯',
        group: "Calculus",
    },
    Symbol {
        name: "angle",
        glyph: '∠',
        group: "Calculus",
    },
    Symbol {
        name: "perp",
        glyph: '⊥',
        group: "Calculus",
    },
    Symbol {
        name: "parallel",
        glyph: '∥',
        group: "Calculus",
    },
];

/// The symbols whose name starts with `query`, most relevant first.
/// Prefix matching rather than fuzzy: a reader typing `in` means `in`,
/// and a fuzzy match would bury it under `infty` and `sin`.
pub fn matching(query: &str) -> Vec<&'static Symbol> {
    SYMBOLS
        .iter()
        .filter(|symbol| symbol.name == query)
        .chain(
            SYMBOLS
                .iter()
                .filter(|symbol| symbol.name != query && symbol.name.starts_with(query)),
        )
        .collect()
}

/// Finds one fully typed catalog name, without prefix expansion.
pub fn exact(name: &str) -> Option<&'static Symbol> {
    SYMBOLS.iter().find(|symbol| symbol.name == name)
}

/// Useful mathematical-alphanumeric variants for an ASCII or Greek letter.
/// The plain form already has a normal completion row, so this returns only
/// alternate cells for the variant grid.
pub fn variants(glyph: char) -> Vec<Variant> {
    const STYLES: &[(&str, &str, u32, u32)] = &[
        ("bold", "Bold", 0x1d400, 0x1d41a),
        ("bold_italic", "Bold italic", 0x1d468, 0x1d482),
        ("sans", "Sans serif", 0x1d5a0, 0x1d5ba),
        ("sans_bold", "Sans bold", 0x1d5d4, 0x1d5ee),
        ("sans_italic", "Sans italic", 0x1d608, 0x1d622),
        ("sans_bold_italic", "Sans bold italic", 0x1d63c, 0x1d656),
        ("monospace", "Monospace", 0x1d670, 0x1d68a),
    ];
    const GREEK_STYLES: &[(&str, &str, u32, u32)] = &[
        ("bold", "Bold", 0x1d6a8, 0x1d6c2),
        ("italic", "Italic", 0x1d6e2, 0x1d6fc),
        ("bold_italic", "Bold italic", 0x1d71c, 0x1d736),
        ("sans_bold", "Sans bold", 0x1d756, 0x1d770),
        ("sans_bold_italic", "Sans bold italic", 0x1d790, 0x1d7aa),
    ];

    if glyph.is_ascii_alphabetic() {
        let offset = glyph.to_ascii_lowercase() as u32 - 'a' as u32;
        return STYLES
            .iter()
            .filter_map(|&(key, name, upper, lower)| {
                char::from_u32(
                    if glyph.is_ascii_uppercase() {
                        upper
                    } else {
                        lower
                    } + offset,
                )
                .map(|glyph| Variant { key, name, glyph })
            })
            .collect();
    }

    let Some((uppercase, index)) = "ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩ"
        .chars()
        .position(|candidate| candidate == glyph)
        .map(|index| (true, index))
        .or_else(|| {
            "αβγδεζηθικλμνξοπρστυφχψω"
                .chars()
                .position(|candidate| candidate == glyph)
                .map(|index| (false, index))
        })
    else {
        return Vec::new();
    };
    GREEK_STYLES
        .iter()
        .filter_map(|&(key, name, upper, lower)| {
            // Mathematical Greek inserts a theta symbol after capital rho and
            // final sigma after lowercase rho.
            let skipped = usize::from(index >= 17);
            char::from_u32((if uppercase { upper } else { lower }) + (index + skipped) as u32)
                .map(|glyph| Variant { key, name, glyph })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_name_outranks_a_longer_prefix() {
        let matches = matching("in");

        assert_eq!(matches[0].name, "in");
        assert_eq!(matches[1].name, "infty");
    }

    #[test]
    fn case_distinguishes_a_capital() {
        assert_eq!(
            matching("Delta").iter().map(|s| s.name).collect::<Vec<_>>(),
            ["Delta"]
        );
        assert_eq!(
            matching("delta").iter().map(|s| s.name).collect::<Vec<_>>(),
            ["delta"]
        );
    }

    #[test]
    fn every_name_is_unique() {
        for (index, symbol) in SYMBOLS.iter().enumerate() {
            assert!(
                !SYMBOLS[..index]
                    .iter()
                    .any(|previous| previous.name == symbol.name)
            );
        }
    }

    #[test]
    fn exact_names_do_not_expand_prefixes() {
        assert_eq!(exact("x").map(|symbol| symbol.glyph), Some('x'));
        assert!(exact("alph").is_none());
    }

    #[test]
    fn variables_and_greek_get_unicode_variants() {
        assert_eq!(variants('x')[0].glyph as u32, 0x1d431);
        assert_eq!(variants('π')[0].glyph as u32, 0x1d6d1);
        assert!(variants('×').is_empty());
    }
}
