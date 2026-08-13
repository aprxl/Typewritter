//! Canonical linear notation for math trees.
//!
//! Deterministic printing is a correctness requirement: auto-commit compares
//! serialized note state, so non-deterministic output creates phantom commits.
//! The notation has two round-trip properties: `print` followed by `parse` is
//! identity on trees, and `parse` followed by `print` is byte-identity on
//! strings produced by `print`. Structural characters in `Sym` nodes are
//! escaped with a backslash; a trailing backslash is the literal backslash.
//! Format change: notation that used `(` for invisible grouping now uses `{`,
//! while `(` and `[` are visible bracket groups; no migration is provided.

use super::math::{BigOp, MathList, MathNode, Slot, WORDS};

/// The canonical linear form of `list`. Deterministic: equal trees print
/// equal strings, and the parser reads this exact string back to an equal
/// tree.
pub fn print(list: &MathList) -> String {
    let mut output = String::new();
    print_list(list, &mut output);
    output
}

fn print_list(list: &MathList, output: &mut String) {
    for (index, node) in list.iter().enumerate() {
        let needs_group = (index > 0
            && matches!(
                node,
                MathNode::Frac { .. }
                    | MathNode::Script { .. }
                    | MathNode::Sqrt { .. }
                    | MathNode::BigOp { .. }
            ))
            || (index == 0
                && matches!(node, MathNode::Frac { .. } | MathNode::Script { .. })
                && matches!(
                    list.get(index + 1),
                    Some(MathNode::Sym(c)) if c.is_alphabetic()
                ));
        if needs_group {
            escape_keyword_suffix(output);
            output.push('{');
            print_node(node, output);
            output.push('}');
        } else {
            print_node(node, output);
        }
    }
}

fn escape_keyword_suffix(output: &mut String) {
    if let Some(keyword) = WORDS
        .iter()
        .map(|(keyword, _)| *keyword)
        .find(|keyword| output.ends_with(keyword))
    {
        output.insert(output.len() - keyword.len() + keyword.len() - 1, '\\');
    }
}

fn print_node(node: &MathNode, output: &mut String) {
    match node {
        MathNode::Sym(c) => {
            if matches!(
                c,
                '/' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' | '^' | '_'
            ) {
                output.push('\\');
            }
            output.push(*c);
        }
        MathNode::Frac { num, den } => {
            print_operand(num, false, output);
            output.push('/');
            print_operand(den, true, output);
        }
        MathNode::Script { base, sup, sub } => {
            print_operand(base, false, output);
            if let Some(sup) = sup {
                output.push('^');
                print_operand(sup, true, output);
            }
            if let Some(sub) = sub {
                output.push('_');
                print_operand(sub, true, output);
            }
        }
        MathNode::Group { open, close, body } => {
            output.push(*open);
            print_list(body, output);
            output.push(*close);
        }
        MathNode::Sqrt { body } => {
            output.push_str("sqrt");
            output.push('{');
            print_list(body, output);
            output.push('}');
        }
        MathNode::BigOp { kind, lower, upper } => {
            output.push_str(kind.keyword());
            for slot in [Slot::Lower, Slot::Upper] {
                if slot == Slot::Upper && *kind == BigOp::Limit {
                    break;
                }
                output.push('{');
                let list = match slot {
                    Slot::Lower => lower,
                    Slot::Upper => upper,
                    _ => unreachable!("big operators only print limit slots"),
                };
                print_list(list, output);
                output.push('}');
            }
        }
    }
}

fn print_operand(list: &MathList, symbol_only: bool, output: &mut String) {
    let bare = match list.as_slice() {
        [MathNode::Sym(_)] => true,
        [node] if !symbol_only && !matches!(node, MathNode::Script { .. }) => true,
        _ => false,
    };
    if bare {
        print_node(&list[0], output);
    } else {
        output.push('{');
        print_list(list, output);
        output.push('}');
    }
}

/// Parses canonical notation. Total: any input produces a tree — an
/// unmatched `)` or a trailing `\\` is taken literally rather than rejected,
/// because a note file must always open (MATH.md §6). On strings `print`
/// produced this is print's exact inverse.
pub fn parse(text: &str) -> MathList {
    Parser {
        chars: text.chars().collect(),
        position: 0,
    }
    .parse_list(None)
}

struct Parser {
    chars: Vec<char>,
    position: usize,
}

impl Parser {
    fn parse_list(&mut self, close: Option<char>) -> MathList {
        // Preserve invisible grouping through postfix attachment, then discard it.
        let mut items: Vec<(MathList, bool)> = Vec::new();

        while let Some(&current) = self.chars.get(self.position) {
            if close == Some(current) {
                self.position += 1;
                break;
            }

            if current == '/' {
                self.position += 1;
                let num = items.pop().map_or_else(Vec::new, |(list, _)| list);
                let den = if self.starts_operand(close) {
                    self.parse_item().map_or_else(Vec::new, |(list, _)| list)
                } else {
                    Vec::new()
                };
                items.push((vec![MathNode::Frac { num, den }], false));
            } else if matches!(current, '^' | '_') {
                self.position += 1;
                let which = if current == '^' { Slot::Sup } else { Slot::Sub };
                let script = if self.starts_operand(close) {
                    self.parse_item().map_or_else(Vec::new, |(list, _)| list)
                } else {
                    Vec::new()
                };
                attach_or_wrap_script(&mut items, which, script);
            } else if let Some(item) = self.parse_item() {
                items.push(item);
            }
        }

        items.into_iter().flat_map(|(list, _)| list).collect()
    }

    fn starts_operand(&self, close: Option<char>) -> bool {
        match self.chars.get(self.position) {
            None | Some('/' | '^' | '_') => false,
            Some(c) if close == Some(*c) => false,
            _ => true,
        }
    }

    fn parse_item(&mut self) -> Option<(MathList, bool)> {
        let current = *self.chars.get(self.position)?;
        match current {
            '/' => None,
            '^' | '_' => None,
            '{' => {
                self.position += 1;
                Some((self.parse_list(Some('}')), true))
            }
            '(' | '[' => {
                let open = current;
                let close = if open == '(' { ')' } else { ']' };
                self.position += 1;
                Some((
                    vec![MathNode::Group {
                        open,
                        close,
                        body: self.parse_list(Some(close)),
                    }],
                    false,
                ))
            }
            '\\' => {
                self.position += 1;
                let escaped = self.chars.get(self.position).copied().unwrap_or('\\');
                if self.position < self.chars.len() {
                    self.position += 1;
                }
                Some((vec![MathNode::Sym(escaped)], false))
            }
            ')' => {
                self.position += 1;
                Some((vec![MathNode::Sym(')')], false))
            }
            c if c.is_alphabetic() => {
                let start = self.position;
                while self
                    .chars
                    .get(self.position)
                    .is_some_and(|c| c.is_alphabetic())
                {
                    self.position += 1;
                }
                let word: String = self.chars[start..self.position].iter().collect();
                let Some(build) = WORDS
                    .iter()
                    .find(|(keyword, _)| {
                        *keyword == word && self.chars.get(self.position) == Some(&'{')
                    })
                    .map(|(_, build)| *build)
                else {
                    return Some((word.chars().map(MathNode::Sym).collect(), false));
                };
                let mut node = build();
                for slot in node.slots() {
                    let list = if self.chars.get(self.position) == Some(&'{') {
                        self.position += 1;
                        self.parse_list(Some('}'))
                    } else {
                        Vec::new()
                    };
                    *node
                        .slot_mut(slot)
                        .expect("keyword node slots must resolve") = list;
                }
                Some((vec![node], false))
            }
            c => {
                self.position += 1;
                Some((vec![MathNode::Sym(c)], false))
            }
        }
    }
}

fn attach_or_wrap_script(items: &mut Vec<(MathList, bool)>, which: Slot, script: MathList) {
    if let Some(item) = items.last_mut()
        && !item.1
        && let [MathNode::Script { sup, sub, .. }] = item.0.as_mut_slice()
    {
        let target = match which {
            Slot::Sup => sup,
            Slot::Sub => sub,
            Slot::Base | Slot::Num | Slot::Den | Slot::Body | Slot::Lower | Slot::Upper => {
                unreachable!()
            }
        };
        if target.is_none() {
            *target = Some(script);
            return;
        }
    }

    let base = items.pop().map_or_else(Vec::new, |(list, _)| list);
    let (sup, sub) = match which {
        Slot::Sup => (Some(script), None),
        Slot::Sub => (None, Some(script)),
        Slot::Base | Slot::Num | Slot::Den | Slot::Body | Slot::Lower | Slot::Upper => {
            unreachable!()
        }
    };
    items.push((vec![MathNode::Script { base, sup, sub }], false));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(s: &str) -> MathList {
        s.chars().map(MathNode::Sym).collect()
    }

    fn frac(num: MathList, den: MathList) -> MathNode {
        MathNode::Frac { num, den }
    }

    fn script(base: MathList, sup: Option<MathList>, sub: Option<MathList>) -> MathNode {
        MathNode::Script { base, sup, sub }
    }

    fn group(open: char, close: char, body: MathList) -> MathNode {
        MathNode::Group { open, close, body }
    }

    fn sqrt(body: MathList) -> MathNode {
        MathNode::Sqrt { body }
    }

    fn big_op(kind: BigOp, lower: MathList, upper: MathList) -> MathNode {
        MathNode::BigOp { kind, lower, upper }
    }

    #[test]
    fn a_flat_list_prints_as_its_chars() {
        let list = sym("1+x y");

        assert_eq!(print(&list), "1+x y");
        assert_eq!(parse("1+x y"), list);
    }

    #[test]
    fn a_fraction_prints_operands_bare_only_when_single() {
        let fraction = frac(sym("R_2"), sym("R_1"));

        assert_eq!(print(&vec![fraction.clone()]), "{R\\_2}/{R\\_1}");
        assert_eq!(parse("{R\\_2}/{R\\_1}"), vec![fraction]);
        assert_eq!(print(&vec![frac(sym("a"), sym("b"))]), "a/b");
    }

    #[test]
    fn a_script_prints_bare_only_for_a_single_symbol() {
        assert_eq!(print(&vec![script(sym("x"), Some(sym("2")), None)]), "x^2");
        assert_eq!(
            print(&vec![script(sym("x"), Some(sym("2n")), None)]),
            "x^{2n}"
        );
        assert_eq!(
            print(&vec![script(
                sym("x"),
                Some(vec![frac(sym("a"), sym("b"))]),
                None,
            )]),
            "x^{a/b}"
        );
    }

    #[test]
    fn both_scripts_print_and_read_back_as_one_base() {
        let tree = vec![script(sym("x"), Some(sym("2")), Some(sym("i")))];

        assert_eq!(print(&tree), "x^2_i");
        assert_eq!(parse("x^2_i"), tree);
    }

    #[test]
    fn a_literal_caret_is_escaped() {
        let tree = sym("^");

        assert_eq!(print(&tree), "\\^");
        assert_eq!(parse("\\^"), tree);
    }

    #[test]
    fn a_group_prints_its_own_brackets() {
        let parentheses = vec![group('(', ')', sym("1+x"))];
        let brackets = vec![group('[', ']', sym("1+x"))];

        assert_eq!(print(&parentheses), "(1+x)");
        assert_eq!(parse("(1+x)"), parentheses);
        assert_eq!(print(&brackets), "[1+x]");
        assert_eq!(parse("[1+x]"), brackets);
    }

    #[test]
    fn an_operand_groups_invisibly() {
        let sum = vec![frac(sym("1+x"), sym("2"))];
        let bracketed_sum = vec![frac(vec![group('(', ')', sym("1+x"))], sym("2"))];

        assert_eq!(print(&sum), "{1+x}/2");
        assert_eq!(parse("{1+x}/2"), sum);
        assert_eq!(print(&bracketed_sum), "(1+x)/2");
        assert_eq!(parse("(1+x)/2"), bracketed_sum);
    }

    #[test]
    fn a_literal_bracket_is_escaped() {
        let tree = sym("()[]{}");

        assert_eq!(print(&tree), "\\(\\)\\[\\]\\{\\}");
        assert_eq!(parse("\\(\\)\\[\\]\\{\\}"), tree);
    }

    #[test]
    fn empty_slots_print_as_empty_invisible_groups() {
        let list = vec![frac(Vec::new(), Vec::new())];

        assert_eq!(print(&list), "{}/{}");
        assert_eq!(parse("{}/{}"), list);
    }

    #[test]
    fn a_radical_prints_its_braced_body() {
        let list = vec![sqrt(sym("1+x"))];

        assert_eq!(print(&list), "sqrt{1+x}");
        assert_eq!(parse("sqrt{1+x}"), list);
    }

    #[test]
    fn a_big_operator_prints_a_group_per_slot() {
        let sum = vec![big_op(BigOp::Sum, sym("i=0"), sym("n"))];
        let limit = vec![big_op(BigOp::Limit, sym("x"), Vec::new())];
        let empty = vec![big_op(BigOp::Sum, Vec::new(), Vec::new())];

        assert_eq!(print(&sum), "sum{i=0}{n}");
        assert_eq!(parse("sum{i=0}{n}"), sum);
        assert_eq!(print(&limit), "lim{x}");
        assert_eq!(parse("lim{x}"), limit);
        assert_eq!(print(&empty), "sum{}{}");
        assert_eq!(parse("sum{}{}"), empty);
    }

    #[test]
    fn a_keyword_not_followed_by_a_group_is_just_letters() {
        let list = sym("sum");

        assert_eq!(print(&list), "sum");
        assert_eq!(parse("sum"), list);
    }

    #[test]
    fn a_keyword_suffix_before_a_braced_operand_stays_letters() {
        let list = vec![
            group('(', ')', vec![frac(sym("limintsum"), sym("alim"))]),
            frac(Vec::new(), Vec::new()),
        ];

        let printed = print(&list);
        assert_eq!(printed, "({limintsum}/{alim}){{}/{}}");
        assert_eq!(parse(&printed), list);
    }

    #[test]
    fn a_literal_slash_is_escaped() {
        let list = sym("12/34");

        assert_eq!(print(&list), "12\\/34");
        assert_eq!(parse("12\\/34"), list);
    }

    #[test]
    fn nesting_prints_without_redundant_parens() {
        let list = vec![frac(vec![frac(sym("1"), sym("2"))], sym("3"))];

        assert_eq!(print(&list), "1/2/3");
        assert_eq!(parse("1/2/3"), list);
    }

    #[test]
    fn a_denominator_fraction_needs_invisible_grouping() {
        let left_associative = vec![frac(vec![frac(sym("1"), sym("2"))], sym("3"))];
        let denominator_fraction = vec![frac(sym("1"), vec![frac(sym("2"), sym("3"))])];

        assert_eq!(print(&left_associative), "1/2/3");
        assert_eq!(print(&denominator_fraction), "1/{2/3}");
        assert_ne!(print(&left_associative), print(&denominator_fraction));
        assert_eq!(parse(&print(&left_associative)), left_associative);
        assert_eq!(parse(&print(&denominator_fraction)), denominator_fraction);
    }

    #[test]
    fn an_empty_numerator_after_an_atom_keeps_its_invisible_grouping() {
        let list = vec![MathNode::Sym('a'), frac(Vec::new(), sym("b"))];

        let printed = print(&list);
        assert_eq!(printed, "a{{}/b}");
        assert_eq!(parse(&printed), list);
    }

    #[test]
    fn unmatched_grouping_does_not_panic() {
        let parsed = parse("){{a");

        assert_eq!(parsed, vec![MathNode::Sym(')'), MathNode::Sym('a')]);
        assert_eq!(print(&parsed), "\\)a");
        assert_eq!(parse(&print(&parsed)), parsed);

        assert_eq!(parse("(a"), vec![group('(', ')', sym("a"))]);
        assert_eq!(parse(")"), sym(")"));
    }

    #[test]
    fn hand_typed_spaces_survive() {
        let parsed = parse("a / b");
        let printed = print(&parsed);

        assert_eq!(printed, "a{ / }b");
        assert_eq!(parse(&printed), parsed);
        assert_eq!(print(&parse(&printed)), printed);
    }

    #[test]
    fn generated_trees_round_trip_both_ways() {
        let mut generator = Generator(0x5eed_1234_5678_9abc);

        for _ in 0..500 {
            let tree = generated_list(&mut generator, 4);
            let printed = print(&tree);

            assert_eq!(parse(&printed), tree, "printed: {printed:?}");
            assert_eq!(print(&parse(&printed)), printed, "printed: {printed:?}");
        }
    }

    struct Generator(u64);

    impl Generator {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0
        }
    }

    fn generated_list(generator: &mut Generator, depth: usize) -> MathList {
        let length = (generator.next() % 5) as usize;
        (0..length)
            .flat_map(|_| {
                let choice = if depth > 0 {
                    generator.next() % 7
                } else {
                    5 + generator.next() % 2
                };
                match choice {
                    0 => vec![frac(
                        generated_list(generator, depth - 1),
                        generated_list(generator, depth - 1),
                    )],
                    1 => {
                        let sup = if generator.next().is_multiple_of(2) {
                            Some(generated_list(generator, depth - 1))
                        } else {
                            None
                        };
                        let sub = if generator.next().is_multiple_of(2) {
                            Some(generated_list(generator, depth - 1))
                        } else {
                            None
                        };
                        let (sup, sub) = if sup.is_none() && sub.is_none() {
                            (Some(Vec::new()), None)
                        } else {
                            (sup, sub)
                        };
                        vec![script(generated_list(generator, depth - 1), sup, sub)]
                    }
                    2 => {
                        let (open, close) = if generator.next().is_multiple_of(2) {
                            ('(', ')')
                        } else {
                            ('[', ']')
                        };
                        vec![group(open, close, generated_list(generator, depth - 1))]
                    }
                    3 => vec![sqrt(generated_list(generator, depth - 1))],
                    4 => {
                        let kind = match generator.next() % 4 {
                            0 => BigOp::Sum,
                            1 => BigOp::Prod,
                            2 => BigOp::Integral,
                            _ => BigOp::Limit,
                        };
                        let lower = generated_list(generator, depth - 1);
                        let upper = if kind == BigOp::Limit {
                            Vec::new()
                        } else {
                            generated_list(generator, depth - 1)
                        };
                        vec![big_op(kind, lower, upper)]
                    }
                    5 => {
                        const KEYWORDS: &[&str] = &["sqrt", "sum", "prod", "int", "lim"];
                        KEYWORDS[(generator.next() % KEYWORDS.len() as u64) as usize]
                            .chars()
                            .map(MathNode::Sym)
                            .collect()
                    }
                    _ => {
                        const SYMBOLS: &[char] = &[
                            'a', 'Z', '0', '9', '+', '-', '=', '.', ' ', '/', '(', ')', '[', ']',
                            '{', '}', '\\', '^', '_',
                        ];
                        vec![MathNode::Sym(
                            SYMBOLS[(generator.next() % SYMBOLS.len() as u64) as usize],
                        )]
                    }
                }
            })
            .collect()
    }
}
