//! The set of things a reader can reach by name inside an expression, which
//! is what makes notation typable without a symbol picker. Names are the ones
//! a LaTeX-trained reader already has in their fingers, because that is the
//! vocabulary this reader arrives with.

/// One completion offered inside an expression.
pub struct Symbol {
    /// What you type to reach it.
    pub name: &'static str,
    /// What it inserts.
    pub glyph: char,
    /// The heading it is listed under.
    pub group: &'static str,
}

pub const SYMBOLS: &[Symbol] = &[
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
}
