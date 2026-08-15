//! Pure completion queries and explicit rewrites for compact math input.
//!
//! The query is derived from the tree every time, so there is no second input
//! buffer to desynchronise after movement or deletion. Rewrites are only
//! suggestions: the tree changes solely through [`accept`].

use super::{
    math::{self, Completion, MathCursor, MathList, MathNode, Step, SymbolRole},
    math_symbols,
};

/// The alphanumeric token immediately before a math cursor.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Query {
    pub path: Vec<Step>,
    pub start: usize,
    pub end: usize,
    pub source: String,
}

/// One row the in-math completion card can accept.
#[derive(Clone, PartialEq, Debug)]
pub enum Offer {
    /// An existing named symbol or structure completion.
    Named(Completion),
    /// An explicit interpretation of a compact alphanumeric token.
    Rewrite {
        title: String,
        group: &'static str,
        preview: String,
        replacement: MathList,
    },
    /// One visual variant of an exact resolved symbol.
    Variant {
        name: &'static str,
        group: &'static str,
        preview: String,
        replacement: MathList,
    },
}

/// Completion rows plus the alphabetic suffix used to obtain named rows.
#[derive(Clone, PartialEq, Debug)]
pub struct OfferSet {
    pub named_query: Option<String>,
    pub offers: Vec<Offer>,
}

/// Returns the trailing alphanumeric token in the cursor's current list.
pub fn query_before(root: &MathList, cursor: &MathCursor) -> Option<Query> {
    let mut cursor = cursor.clone();
    math::clamp(root, &mut cursor);
    let list = math::list_at(root, &cursor.path)?;
    let end = cursor.index;
    let mut start = end;
    while start > 0 && matches!(list[start - 1], MathNode::Sym(ch) if ch.is_ascii_alphanumeric()) {
        start -= 1;
    }
    if start == end {
        return None;
    }

    Some(Query {
        path: cursor.path,
        start,
        end,
        source: symbols(&list[start..end])?,
    })
}

/// Returns known identities, generic compact-script interpretations, then
/// completions for the trailing named suffix.
pub fn offers(query: &Query) -> OfferSet {
    let named_query = trailing_letters(&query.source);
    let (rewrites, mut generic) = rewrites(&query.source);
    let variants = variant_offers(named_query.as_deref(), &rewrites, &generic);
    let mut named: (Vec<_>, Vec<_>) = named_query
        .as_deref()
        .map(math::completions)
        .unwrap_or_default()
        .into_iter()
        .filter(|completion| !duplicates_role_choice(completion, query, named_query.as_deref()))
        .partition(|completion| is_exact(completion, named_query.as_deref()));

    let mut offers = rewrites;
    offers.extend(named.0.drain(..).map(Offer::Named));
    offers.append(&mut generic);
    offers.extend(named.1.drain(..).map(Offer::Named));
    offers.extend(variants);
    OfferSet {
        named_query,
        offers,
    }
}

/// Applies an offered row only while its original query still names the text
/// under the current cursor. Returns `false` without touching tree or cursor
/// when either the query or offer is stale.
pub fn accept(root: &mut MathList, cursor: &mut MathCursor, query: &Query, offer: &Offer) -> bool {
    if !query_is_current(root, cursor, query) || !offers(query).offers.contains(offer) {
        return false;
    }

    match offer {
        Offer::Named(Completion::Symbol { name, glyph, .. }) => {
            let suffix_len = trailing_letters(&query.source)
                .expect("a named symbol offer must have a letter suffix")
                .len();
            let start = query.end - suffix_len;
            let list = math::list_at_mut(root, &query.path)
                .expect("a current conversion query path must resolve");
            let replacement = math_symbols::exact(name)
                .and_then(|symbol| symbol.role())
                .map_or(MathNode::Sym(*glyph), |role| {
                    resolved(name, role, "plain", sym(&glyph.to_string()))
                });
            list.splice(start..query.end, [replacement]);
            cursor.path.clone_from(&query.path);
            cursor.index = start + 1;
            true
        }
        Offer::Named(Completion::Structure { name, .. }) => {
            math::insert_structure(root, cursor, name)
        }
        Offer::Rewrite { replacement, .. } => {
            let list = math::list_at_mut(root, &query.path)
                .expect("a current conversion query path must resolve");
            list.splice(query.start..query.end, replacement.clone());
            cursor.path.clone_from(&query.path);
            cursor.index = query.start + replacement.len();
            true
        }
        Offer::Variant { replacement, .. } => {
            let suffix_len = trailing_letters(&query.source)
                .expect("a variant offer must have an exact letter suffix")
                .len();
            let start = query.end - suffix_len;
            let list = math::list_at_mut(root, &query.path)
                .expect("a current conversion query path must resolve");
            list.splice(start..query.end, replacement.clone());
            cursor.path.clone_from(&query.path);
            cursor.index = start + replacement.len();
            true
        }
    }
}

fn query_is_current(root: &MathList, cursor: &MathCursor, query: &Query) -> bool {
    if cursor.path != query.path || cursor.index != query.end || query.start >= query.end {
        return false;
    }
    let Some(list) = math::list_at(root, &query.path) else {
        return false;
    };
    list.get(query.start..query.end)
        .and_then(symbols)
        .is_some_and(|source| source == query.source)
}

fn symbols(nodes: &[MathNode]) -> Option<String> {
    nodes
        .iter()
        .map(|node| match node {
            MathNode::Sym(ch) => Some(*ch),
            _ => None,
        })
        .collect()
}

fn trailing_letters(source: &str) -> Option<String> {
    let letters: String = source
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_alphabetic())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (!letters.is_empty()).then_some(letters)
}

fn rewrites(source: &str) -> (Vec<Offer>, Vec<Offer>) {
    let mut offers: Vec<Offer> = known_constant(source).into_iter().collect();
    offers.extend(known_function(source));
    let has_constant = offers
        .iter()
        .any(|offer| rewrite_role(offer) == Some(SymbolRole::Constant));
    let has_function = offers
        .iter()
        .any(|offer| rewrite_role(offer) == Some(SymbolRole::Function));
    let roles = role_choices(source, has_constant, has_function);

    if let Some((base, digits, tail)) = compact_script(source) {
        offers.push(rewrite(
            format!("{base} power {digits}"),
            "Interpretation",
            format!("{base}{}{tail}", raised(digits, true)),
            script(base, Some(digits), None, tail),
        ));
        offers.push(rewrite(
            format!("{base} index {digits}"),
            "Interpretation",
            format!("{base}{}{tail}", raised(digits, false)),
            script(base, None, Some(digits), tail),
        ));
    }

    (offers, roles)
}

fn duplicates_role_choice(
    completion: &Completion,
    query: &Query,
    named_query: Option<&str>,
) -> bool {
    let Completion::Symbol { name, .. } = completion else {
        return false;
    };
    named_query == Some(query.source.as_str())
        && *name == query.source.as_str()
        && math_symbols::exact(name).is_some_and(|symbol| symbol.role().is_some())
}

fn is_exact(completion: &Completion, named_query: Option<&str>) -> bool {
    match completion {
        Completion::Symbol { name, .. } | Completion::Structure { name, .. } => {
            Some(*name) == named_query
        }
    }
}

fn rewrite_role(offer: &Offer) -> Option<SymbolRole> {
    let Offer::Rewrite { replacement, .. } = offer else {
        return None;
    };
    match replacement.as_slice() {
        [MathNode::Resolved { role, .. }] => Some(*role),
        _ => None,
    }
}

fn role_choices(source: &str, has_constant: bool, has_function: bool) -> Vec<Offer> {
    if source.is_empty() || !source.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return Vec::new();
    }

    let body = math_symbols::exact(source)
        .filter(|symbol| symbol.role().is_some())
        .map_or_else(|| sym(source), |symbol| sym(&symbol.glyph.to_string()));
    [
        (SymbolRole::Variable, "Variable", false),
        (SymbolRole::Constant, "Constant", has_constant),
        (SymbolRole::Function, "Function", has_function),
    ]
    .into_iter()
    .filter(|(_, _, already_known)| !already_known)
    .map(|(role, group, _)| {
        rewrite(
            format!("{source} as {}", role.keyword()),
            group,
            symbols(&body).expect("a role-choice body contains symbols"),
            vec![resolved(source, role, "plain", body.clone())],
        )
    })
    .collect()
}

fn known_function(source: &str) -> Option<Offer> {
    let (id, title) = function_name(source)?;
    Some(rewrite(
        title,
        "Function",
        source,
        vec![resolved(id, SymbolRole::Function, "plain", sym(source))],
    ))
}

fn function_name(source: &str) -> Option<(&'static str, &'static str)> {
    Some(match source {
        "f" => ("f", "Function f"),
        "g" => ("g", "Function g"),
        "sin" => ("sin", "Sine"),
        "cos" => ("cos", "Cosine"),
        "tan" => ("tan", "Tangent"),
        "log" => ("log", "Logarithm"),
        "ln" => ("ln", "Natural logarithm"),
        "exp" => ("exp", "Exponential"),
        _ => return None,
    })
}

fn known_constant(source: &str) -> Option<Offer> {
    let (id, title, preview, body) = match source {
        "pi" => ("pi", "Pi", "π", sym("π")),
        "tau" => ("tau", "Tau", "τ", sym("τ")),
        "e" | "euler" => ("euler_number", "Euler's number", "e", sym("e")),
        "i" | "iunit" => ("imaginary_unit", "Imaginary unit", "i", sym("i")),
        "golden" | "goldenratio" => ("golden_ratio", "Golden ratio", "φ", sym("φ")),
        "e0" | "eps0" | "epsilon0" => (
            "vacuum_permittivity",
            "Vacuum permittivity",
            "ε₀",
            script("ε", None, Some("0"), ""),
        ),
        "mu0" => (
            "vacuum_permeability",
            "Vacuum permeability",
            "μ₀",
            script("μ", None, Some("0"), ""),
        ),
        "hbar" => (
            "reduced_planck_constant",
            "Reduced Planck constant",
            "ℏ",
            sym("ℏ"),
        ),
        "kB" => (
            "boltzmann_constant",
            "Boltzmann constant",
            "k_B",
            script("k", None, Some("B"), ""),
        ),
        "NA" => (
            "avogadro_constant",
            "Avogadro constant",
            "N_A",
            script("N", None, Some("A"), ""),
        ),
        "qe" => (
            "elementary_charge",
            "Elementary charge",
            "qₑ",
            script("q", None, Some("e"), ""),
        ),
        "me" => (
            "electron_mass",
            "Electron mass",
            "mₑ",
            script("m", None, Some("e"), ""),
        ),
        "mp" => (
            "proton_mass",
            "Proton mass",
            "mₚ",
            script("m", None, Some("p"), ""),
        ),
        "mn" => (
            "neutron_mass",
            "Neutron mass",
            "mₙ",
            script("m", None, Some("n"), ""),
        ),
        "a0" => (
            "bohr_radius",
            "Bohr radius",
            "a₀",
            script("a", None, Some("0"), ""),
        ),
        "Rinf" => (
            "rydberg_constant",
            "Rydberg constant",
            "R_∞",
            script("R", None, Some("∞"), ""),
        ),
        "sigmaSB" => (
            "stefan_boltzmann_constant",
            "Stefan–Boltzmann constant",
            "σ_SB",
            script("σ", None, Some("SB"), ""),
        ),
        "lambdaC" => (
            "compton_wavelength",
            "Compton wavelength",
            "λ_C",
            script("λ", None, Some("C"), ""),
        ),
        "c" => ("speed_of_light", "Speed of light", "c", sym("c")),
        "G" => (
            "gravitational_constant",
            "Gravitational constant",
            "G",
            sym("G"),
        ),
        "h" => ("planck_constant", "Planck constant", "h", sym("h")),
        "R" => ("molar_gas_constant", "Molar gas constant", "R", sym("R")),
        "F" => ("faraday_constant", "Faraday constant", "F", sym("F")),
        _ => return None,
    };
    Some(rewrite(
        title,
        "Constant",
        preview,
        vec![resolved(id, SymbolRole::Constant, "plain", body)],
    ))
}

fn variant_offers(named_query: Option<&str>, rewrites: &[Offer], generic: &[Offer]) -> Vec<Offer> {
    let Some(name) = named_query else {
        return Vec::new();
    };
    for rewrite in rewrites.iter().chain(generic) {
        let Offer::Rewrite { replacement, .. } = rewrite else {
            continue;
        };
        let [MathNode::Resolved { id, role, body, .. }] = replacement.as_slice() else {
            continue;
        };
        return variants(id, *role, body);
    }

    if let Some((symbol, role)) =
        math_symbols::exact(name).and_then(|symbol| symbol.role().map(|role| (symbol, role)))
    {
        return variants(name, role, &sym(&symbol.glyph.to_string()));
    }
    Vec::new()
}

fn variants(id: &str, role: SymbolRole, body: &MathList) -> Vec<Offer> {
    let glyphs: Vec<char> = body
        .iter()
        .map(|node| match node {
            MathNode::Sym(glyph) => Some(*glyph),
            _ => None,
        })
        .collect::<Option<_>>()
        .unwrap_or_default();
    let Some(&first) = glyphs.first() else {
        return Vec::new();
    };
    math_symbols::variants(first)
        .into_iter()
        .filter_map(|variant| {
            let body: MathList = glyphs
                .iter()
                .map(|glyph| {
                    math_symbols::variants(*glyph)
                        .into_iter()
                        .find(|candidate| candidate.key == variant.key)
                        .map(|candidate| MathNode::Sym(candidate.glyph))
                })
                .collect::<Option<_>>()?;
            Some(Offer::Variant {
                name: variant.name,
                group: match role {
                    SymbolRole::Variable => "Variable",
                    SymbolRole::Constant => "Constant",
                    SymbolRole::Function => "Function",
                },
                preview: symbols(&body).expect("a variant body contains symbols"),
                replacement: vec![resolved(id, role, variant.key, body)],
            })
        })
        .collect()
}

fn resolved(id: &str, role: SymbolRole, variant: &str, body: MathList) -> MathNode {
    MathNode::Resolved {
        id: id.to_owned(),
        role,
        variant: variant.to_owned(),
        body,
    }
}

fn rewrite(
    title: impl Into<String>,
    group: &'static str,
    preview: impl Into<String>,
    replacement: MathList,
) -> Offer {
    Offer::Rewrite {
        title: title.into(),
        group,
        preview: preview.into(),
        replacement,
    }
}

fn compact_script(source: &str) -> Option<(&str, &str, &str)> {
    let digit_start = source.find(|ch: char| ch.is_ascii_digit())?;
    if digit_start == 0
        || !source[..digit_start]
            .chars()
            .all(|ch| ch.is_ascii_alphabetic())
    {
        return None;
    }
    let digit_end = source[digit_start..]
        .find(|ch: char| !ch.is_ascii_digit())
        .map_or(source.len(), |offset| digit_start + offset);
    let tail = &source[digit_end..];
    tail.chars().all(|ch| ch.is_ascii_alphabetic()).then_some((
        &source[..digit_start],
        &source[digit_start..digit_end],
        tail,
    ))
}

fn raised(digits: &str, superscript: bool) -> String {
    const SUP: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    const SUB: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];
    let alphabet = if superscript { &SUP } else { &SUB };
    digits
        .bytes()
        .map(|digit| alphabet[(digit - b'0') as usize])
        .collect()
}

fn script(base: &str, sup: Option<&str>, sub: Option<&str>, tail: &str) -> MathList {
    let mut result = vec![MathNode::Script {
        base: sym(base),
        sup: sup.map(sym),
        sub: sub.map(sym),
    }];
    result.extend(sym(tail));
    result
}

fn sym(text: &str) -> MathList {
    text.chars().map(MathNode::Sym).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::math::{Slot, Step};
    use crate::document::math_notation;

    fn at(index: usize) -> MathCursor {
        MathCursor {
            path: Vec::new(),
            index,
        }
    }

    fn query(source: &str) -> Query {
        query_before(&sym(source), &at(source.chars().count())).expect("query")
    }

    fn replacements(source: &str) -> Vec<MathList> {
        offers(&query(source))
            .offers
            .into_iter()
            .filter_map(|offer| match offer {
                Offer::Rewrite { replacement, .. } => Some(replacement),
                Offer::Named(_) | Offer::Variant { .. } => None,
            })
            .collect()
    }

    #[test]
    fn extraction_stops_at_non_alphanumeric_and_structural_nodes() {
        let root = vec![
            MathNode::Sym('a'),
            MathNode::Sym('+'),
            MathNode::Sqrt { body: Vec::new() },
            MathNode::Sym('e'),
            MathNode::Sym('2'),
            MathNode::Sym('x'),
            MathNode::Sym(' '),
        ];

        assert_eq!(
            query_before(&root, &at(6)),
            Some(Query {
                path: Vec::new(),
                start: 3,
                end: 6,
                source: "e2x".into(),
            })
        );
        assert_eq!(query_before(&root, &at(7)), None);
    }

    #[test]
    fn extraction_uses_the_nested_cursor_slot() {
        let root = vec![MathNode::Frac {
            num: sym("a"),
            den: sym("e2x"),
        }];
        let cursor = MathCursor {
            path: vec![Step {
                index: 0,
                slot: Slot::Den,
            }],
            index: 3,
        };

        assert_eq!(
            query_before(&root, &cursor),
            Some(Query {
                path: cursor.path,
                start: 0,
                end: 3,
                source: "e2x".into(),
            })
        );
    }

    #[test]
    fn e0_offers_constant_power_then_index() {
        let found = offers(&query("e0"));
        let titles: Vec<&str> = found
            .offers
            .iter()
            .map(|offer| match offer {
                Offer::Rewrite { title, .. } => title.as_str(),
                Offer::Named(_) | Offer::Variant { .. } => panic!("expected rewrite"),
            })
            .collect();

        assert_eq!(found.named_query, None);
        assert_eq!(titles, ["Vacuum permittivity", "e power 0", "e index 0"]);
        assert_eq!(
            replacements("e0"),
            [
                vec![resolved(
                    "vacuum_permittivity",
                    SymbolRole::Constant,
                    "plain",
                    script("ε", None, Some("0"), ""),
                )],
                script("e", Some("0"), None, ""),
                script("e", None, Some("0"), ""),
            ]
        );
    }

    #[test]
    fn e2x_offers_power_then_index_with_an_implicit_tail() {
        assert_eq!(
            replacements("e2x"),
            [
                script("e", Some("2"), None, "x"),
                script("e", None, Some("2"), "x"),
            ]
        );
    }

    #[test]
    fn any_single_letter_digit_run_gets_script_interpretations() {
        assert_eq!(
            replacements("alpha17beta"),
            [
                script("alpha", Some("17"), None, "beta"),
                script("alpha", None, Some("17"), "beta"),
            ]
        );
        assert!(replacements("2x").is_empty());
        assert!(replacements("x2y3").is_empty());
    }

    #[test]
    fn x2_offers_power_then_index() {
        assert_eq!(
            replacements("x2"),
            [
                script("x", Some("2"), None, ""),
                script("x", None, Some("2"), ""),
            ]
        );
    }

    #[test]
    fn non_recipe_tokens_expose_the_existing_named_query() {
        let alpha = offers(&query("alpha"));
        assert_eq!(alpha.named_query.as_deref(), Some("alpha"));
        assert!(matches!(
            alpha.offers.first(),
            Some(Offer::Rewrite {
                group: "Variable",
                preview,
                ..
            }) if preview == "α"
        ));
        assert!(!alpha.offers.iter().any(|offer| matches!(
            offer,
            Offer::Named(Completion::Symbol { name: "alpha", .. })
        )));

        let frac = offers(&query("1frac"));
        assert_eq!(frac.named_query.as_deref(), Some("frac"));
        assert!(matches!(
            frac.offers.first(),
            Some(Offer::Named(Completion::Structure { name: "frac", .. }))
        ));
    }

    #[test]
    fn known_constants_rank_before_generic_interpretations() {
        let found = offers(&query("mu0"));
        let titles: Vec<&str> = found
            .offers
            .iter()
            .filter_map(|offer| match offer {
                Offer::Rewrite { title, .. } => Some(title.as_str()),
                Offer::Named(_) | Offer::Variant { .. } => None,
            })
            .collect();

        assert_eq!(titles, ["Vacuum permeability", "mu power 0", "mu index 0"]);
        assert_eq!(
            replacements("mu0")[0],
            vec![resolved(
                "vacuum_permeability",
                SymbolRole::Constant,
                "plain",
                script("μ", None, Some("0"), ""),
            )]
        );
    }

    #[test]
    fn stale_query_or_foreign_offer_never_mutates() {
        let mut root = sym("e1");
        let mut cursor = at(2);
        let stale = query("e0");
        let offer = offers(&stale).offers[0].clone();
        let before = root.clone();

        assert!(!accept(&mut root, &mut cursor, &stale, &offer));
        assert_eq!(root, before);
        assert_eq!(cursor, at(2));

        let current = query_before(&root, &cursor).expect("current query");
        assert!(!accept(&mut root, &mut cursor, &current, &offer));
        assert_eq!(root, before);
        assert_eq!(cursor, at(2));
    }

    #[test]
    fn acceptance_preserves_surroundings_and_lands_after_replacement() {
        let mut root = sym("a+e2x+b");
        let mut cursor = at(5);
        let query = query_before(&root, &cursor).expect("e2x query");
        let offer = offers(&query).offers[0].clone();

        assert!(accept(&mut root, &mut cursor, &query, &offer));

        let mut expected = sym("a+");
        expected.extend(script("e", Some("2"), None, "x"));
        expected.extend(sym("+b"));
        assert_eq!(root, expected);
        assert_eq!(cursor, at(4));
    }

    #[test]
    fn accepted_unicode_symbol_starts_a_fresh_adjacent_named_query() {
        let mut root = sym("pi");
        let mut cursor = at(2);
        let pi_query = query_before(&root, &cursor).expect("pi query");
        let pi = offers(&pi_query)
            .offers
            .into_iter()
            .find(|offer| {
                matches!(
                    offer,
                    Offer::Rewrite {
                        group: "Variable",
                        ..
                    }
                )
            })
            .expect("pi variable interpretation");
        assert!(accept(&mut root, &mut cursor, &pi_query, &pi));

        root.extend(sym("theta"));
        cursor.index += 5;
        let theta_query = query_before(&root, &cursor).expect("theta query");
        assert_eq!(theta_query.source, "theta");
        let theta = offers(&theta_query)
            .offers
            .into_iter()
            .find(|offer| {
                matches!(
                    offer,
                    Offer::Rewrite {
                        group: "Variable",
                        ..
                    }
                )
            })
            .expect("theta variable interpretation");
        assert!(accept(&mut root, &mut cursor, &theta_query, &theta));
        assert_eq!(
            root,
            vec![
                resolved("pi", SymbolRole::Variable, "plain", sym("π")),
                resolved("theta", SymbolRole::Variable, "plain", sym("θ")),
            ]
        );
        assert_eq!(cursor, at(2));
    }

    #[test]
    fn variants_follow_normal_rows_and_accept_as_one_resolved_atom() {
        let query = query("x");
        let found = offers(&query);
        let first_variant = found
            .offers
            .iter()
            .position(|offer| matches!(offer, Offer::Variant { .. }))
            .expect("variant rows");
        assert!(
            found.offers[..first_variant]
                .iter()
                .all(|offer| !matches!(offer, Offer::Variant { .. }))
        );

        let mut root = sym("x");
        let mut cursor = at(1);
        let offer = found.offers[first_variant].clone();
        assert!(accept(&mut root, &mut cursor, &query, &offer));
        assert!(matches!(
            root.as_slice(),
            [MathNode::Resolved {
                id,
                role: SymbolRole::Variable,
                variant,
                ..
            }] if id == "x" && variant == "bold"
        ));
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn constant_variants_belong_to_the_preferred_constant() {
        let found = offers(&query("pi"));
        let variants: Vec<(&str, &str)> = found
            .offers
            .iter()
            .filter_map(|offer| match offer {
                Offer::Variant { group, preview, .. } => Some((*group, preview.as_str())),
                Offer::Named(_) | Offer::Rewrite { .. } => None,
            })
            .collect();

        assert_eq!(variants.len(), math_symbols::variants('π').len());
        assert!(variants.iter().all(|(group, _)| *group == "Constant"));
        for (index, (_, preview)) in variants.iter().enumerate() {
            assert!(!variants[..index].iter().any(|(_, other)| other == preview));
        }
    }

    #[test]
    fn compact_function_interpretations_rank_before_variables_and_own_variants() {
        for source in ["f", "g", "sin", "cos", "tan", "log", "ln", "exp"] {
            let found = offers(&query(source));
            assert!(matches!(
                found.offers.first(),
                Some(Offer::Rewrite {
                    group: "Function",
                    ..
                })
            ));
            let mut root = sym(source);
            let mut cursor = at(source.len());
            assert!(accept(
                &mut root,
                &mut cursor,
                &query(source),
                &found.offers[0]
            ));
            assert!(matches!(
                root.as_slice(),
                [MathNode::Resolved {
                    role: SymbolRole::Function,
                    ..
                }]
            ));

            assert!(found.offers.iter().any(|offer| {
                matches!(
                    offer,
                    Offer::Rewrite {
                        group: "Variable",
                        ..
                    }
                )
            }));
            assert!(found.offers.iter().any(|offer| {
                matches!(
                    offer,
                    Offer::Rewrite {
                        group: "Constant",
                        ..
                    }
                )
            }));

            let variants: Vec<&str> = found
                .offers
                .iter()
                .filter_map(|offer| match offer {
                    Offer::Variant { group, .. } => Some(*group),
                    Offer::Named(_) | Offer::Rewrite { .. } => None,
                })
                .collect();
            assert!(!variants.is_empty());
            assert!(variants.iter().all(|group| *group == "Function"));
        }
    }

    #[test]
    fn x_offers_three_ranked_roles_and_accepts_each_as_persistent_identity() {
        let query = query("x");
        let found = offers(&query);
        let role_rows: Vec<_> = found
            .offers
            .iter()
            .filter_map(|offer| rewrite_role(offer).map(|role| (offer, role)))
            .collect();

        assert_eq!(
            role_rows.iter().map(|(_, role)| *role).collect::<Vec<_>>(),
            [
                SymbolRole::Variable,
                SymbolRole::Constant,
                SymbolRole::Function
            ]
        );
        for ((offer, role), (title, group)) in role_rows.iter().zip([
            ("x as variable", "Variable"),
            ("x as constant", "Constant"),
            ("x as function", "Function"),
        ]) {
            assert!(matches!(
                offer,
                Offer::Rewrite {
                    title: found_title,
                    group: found_group,
                    preview,
                    ..
                } if found_title == title && *found_group == group && preview == "x"
            ));

            let mut root = sym("x");
            let mut cursor = at(1);
            assert!(accept(&mut root, &mut cursor, &query, offer));
            assert_eq!(
                root,
                vec![resolved("x", *role, "plain", sym("x"))],
                "{group} choice must persist its role"
            );
            let notation = math_notation::print(&root);
            assert_eq!(math_notation::parse(&notation), root);
            assert_eq!(cursor, at(1));
        }
    }

    #[test]
    fn known_identity_replaces_only_its_generic_role_and_stays_first() {
        for (source, expected) in [
            (
                "c",
                [
                    SymbolRole::Constant,
                    SymbolRole::Variable,
                    SymbolRole::Function,
                ],
            ),
            (
                "f",
                [
                    SymbolRole::Function,
                    SymbolRole::Variable,
                    SymbolRole::Constant,
                ],
            ),
        ] {
            let found = offers(&query(source));
            let roles: Vec<_> = found.offers.iter().filter_map(rewrite_role).collect();
            assert_eq!(roles, expected, "{source}");
        }
    }

    #[test]
    fn every_rewrite_round_trips_through_canonical_notation() {
        for source in ["e0", "e2x", "x2", "alpha17beta", "mu0", "sigmaSB"] {
            for replacement in replacements(source) {
                let printed = math_notation::print(&replacement);
                assert_eq!(
                    math_notation::parse(&printed),
                    replacement,
                    "{source}: {printed}"
                );
            }
        }
    }
}
