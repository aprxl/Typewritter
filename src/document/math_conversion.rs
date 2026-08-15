//! Pure completion queries and explicit rewrites for compact math input.
//!
//! The query is derived from the tree every time, so there is no second input
//! buffer to desynchronise after movement or deletion. Rewrites are only
//! suggestions: the tree changes solely through [`accept`].

use super::math::{self, Completion, MathCursor, MathList, MathNode, Step};

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
        title: &'static str,
        group: &'static str,
        preview: &'static str,
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
    while start > 0 && matches!(list[start - 1], MathNode::Sym(ch) if ch.is_alphanumeric()) {
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

/// Returns named completions or the explicitly supported compact rewrites.
///
/// An explicit recipe owns the whole token and suppresses named suffix rows;
/// otherwise the existing completion table receives the trailing letter run.
pub fn offers(query: &Query) -> OfferSet {
    let rewrites = rewrites(&query.source);
    if !rewrites.is_empty() {
        return OfferSet {
            named_query: None,
            offers: rewrites,
        };
    }

    let named_query = trailing_letters(&query.source);
    let offers = named_query
        .as_deref()
        .map(math::completions)
        .unwrap_or_default()
        .into_iter()
        .map(Offer::Named)
        .collect();
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
        Offer::Named(Completion::Symbol { glyph, .. }) => {
            math::accept_symbol(root, cursor, *glyph);
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
        .take_while(|ch| ch.is_alphabetic())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (!letters.is_empty()).then_some(letters)
}

fn rewrites(source: &str) -> Vec<Offer> {
    match source {
        "e0" => vec![
            rewrite(
                "Vacuum permittivity",
                "Constant",
                "ε₀",
                script("ε", None, Some("0"), ""),
            ),
            rewrite(
                "e power 0",
                "Interpretation",
                "e⁰",
                script("e", Some("0"), None, ""),
            ),
            rewrite(
                "e index 0",
                "Interpretation",
                "e₀",
                script("e", None, Some("0"), ""),
            ),
        ],
        "e2x" => vec![
            rewrite(
                "e squared × x",
                "Interpretation",
                "e²x",
                script("e", Some("2"), None, "x"),
            ),
            rewrite(
                "e index 2 × x",
                "Interpretation",
                "e₂x",
                script("e", None, Some("2"), "x"),
            ),
        ],
        "x2" => vec![
            rewrite(
                "x squared",
                "Interpretation",
                "x²",
                script("x", Some("2"), None, ""),
            ),
            rewrite(
                "x index 2",
                "Interpretation",
                "x₂",
                script("x", None, Some("2"), ""),
            ),
        ],
        _ => Vec::new(),
    }
}

fn rewrite(
    title: &'static str,
    group: &'static str,
    preview: &'static str,
    replacement: MathList,
) -> Offer {
    Offer::Rewrite {
        title,
        group,
        preview,
        replacement,
    }
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
            .map(|offer| match offer {
                Offer::Rewrite { replacement, .. } => replacement,
                Offer::Named(_) => panic!("recipe unexpectedly produced a named completion"),
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
                Offer::Rewrite { title, .. } => *title,
                Offer::Named(_) => panic!("expected rewrite"),
            })
            .collect();

        assert_eq!(found.named_query, None);
        assert_eq!(titles, ["Vacuum permittivity", "e power 0", "e index 0"]);
        assert_eq!(
            replacements("e0"),
            [
                script("ε", None, Some("0"), ""),
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
            Some(Offer::Named(Completion::Symbol {
                name: "alpha",
                glyph: 'α',
                ..
            }))
        ));

        let frac = offers(&query("1frac"));
        assert_eq!(frac.named_query.as_deref(), Some("frac"));
        assert!(matches!(
            frac.offers.first(),
            Some(Offer::Named(Completion::Structure { name: "frac", .. }))
        ));
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
    fn every_rewrite_round_trips_through_canonical_notation() {
        for source in ["e0", "e2x", "x2"] {
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
