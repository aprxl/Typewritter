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

use super::math::{BigOp, MathList, MathNode, PAIRS, Slot, SymbolRole, WORDS};

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
        let self_paired = self_paired_delimiter(node);
        let needs_group = (index > 0
            && matches!(
                node,
                MathNode::Resolved { .. }
                    | MathNode::Frac { .. }
                    | MathNode::Script { .. }
                    | MathNode::Sqrt { .. }
                    | MathNode::Accent { .. }
                    | MathNode::BigOp { .. }
            ))
            || (index == 0
                && matches!(node, MathNode::Frac { .. } | MathNode::Script { .. })
                && matches!(
                    list.get(index + 1),
                    Some(MathNode::Sym(c)) if c.is_alphabetic()
                ))
            || self_paired.is_some_and(|delimiter| {
                index > 0 && self_paired_delimiter(&list[index - 1]) == Some(delimiter)
            });
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

fn self_paired_delimiter(node: &MathNode) -> Option<char> {
    match node {
        MathNode::Group { open, close, .. } if open == close => Some(*open),
        _ => None,
    }
}

fn contains_unescaped(text: &str, target: char) -> bool {
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == target {
            return true;
        }
    }
    false
}

fn escape_keyword_suffix(output: &mut String) {
    if WORDS
        .iter()
        .map(|(keyword, _)| *keyword)
        .chain(std::iter::once("sym"))
        .any(|keyword| output.ends_with(keyword))
    {
        output.insert(output.len() - 1, '\\');
    }
}

fn print_node(node: &MathNode, output: &mut String) {
    match node {
        MathNode::Sym(c) => {
            if PAIRS.iter().any(|&(open, close)| *c == open || *c == close)
                || matches!(c, '/' | '{' | '}' | '\\' | '^' | '_')
            {
                output.push('\\');
            }
            output.push(*c);
        }
        MathNode::Resolved {
            id,
            role,
            variant,
            body,
        } => {
            output.push_str("sym{");
            output.push_str(role.keyword());
            output.push('|');
            print_header(id, output);
            output.push('|');
            print_header(variant, output);
            output.push_str("}{");
            print_list(body, output);
            output.push('}');
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
            let mut body_output = String::new();
            print_list(body, &mut body_output);
            if open == close && contains_unescaped(&body_output, *open) {
                output.push('{');
                output.push_str(&body_output);
                output.push('}');
            } else {
                output.push_str(&body_output);
            }
            output.push(*close);
        }
        MathNode::Sqrt { body } => {
            output.push_str("sqrt");
            output.push('{');
            print_list(body, output);
            output.push('}');
        }
        MathNode::Accent { kind, body } => {
            output.push_str(kind.keyword());
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
        [MathNode::Sym(_) | MathNode::Resolved { .. }] => true,
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

fn print_header(value: &str, output: &mut String) {
    for c in value.chars() {
        if matches!(c, '\\' | '|' | '{' | '}') {
            output.push('\\');
        }
        output.push(c);
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
            c if PAIRS.iter().any(|&(open, _)| open == c) => {
                let open = c;
                let close = PAIRS
                    .iter()
                    .find(|&&(candidate, _)| candidate == open)
                    .map(|&(_, close)| close)
                    .expect("pair opener must have a closer");
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
                if word == "sym"
                    && self.chars.get(self.position) == Some(&'{')
                    && let Some(node) = self.parse_resolved()
                {
                    return Some((vec![node], false));
                }
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

    fn parse_resolved(&mut self) -> Option<MathNode> {
        let checkpoint = self.position;
        let Some(fields) = self.parse_header() else {
            self.position = checkpoint;
            return None;
        };
        let Some(role) = SymbolRole::from_keyword(&fields[0]) else {
            self.position = checkpoint;
            return None;
        };
        if self.chars.get(self.position) != Some(&'{') {
            self.position = checkpoint;
            return None;
        }
        self.position += 1;
        let body = self.parse_list(Some('}'));
        Some(MathNode::Resolved {
            id: fields[1].clone(),
            role,
            variant: fields[2].clone(),
            body,
        })
    }

    fn parse_header(&mut self) -> Option<[String; 3]> {
        if self.chars.get(self.position) != Some(&'{') {
            return None;
        }
        self.position += 1;
        let mut fields = [String::new(), String::new(), String::new()];
        let mut field = 0;
        while let Some(c) = self.chars.get(self.position).copied() {
            self.position += 1;
            match c {
                '\\' => {
                    let escaped = self.chars.get(self.position).copied()?;
                    self.position += 1;
                    fields[field].push(escaped);
                }
                '|' if field < 2 => field += 1,
                '|' => return None,
                '}' => return (field == 2).then_some(fields),
                _ => fields[field].push(c),
            }
        }
        None
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
    use crate::document::math::AccentKind;

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

    fn accent(kind: AccentKind, body: MathList) -> MathNode {
        MathNode::Accent { kind, body }
    }

    fn big_op(kind: BigOp, lower: MathList, upper: MathList) -> MathNode {
        MathNode::BigOp { kind, lower, upper }
    }

    fn resolved(id: &str, role: SymbolRole, variant: &str, body: MathList) -> MathNode {
        MathNode::Resolved {
            id: id.to_owned(),
            role,
            variant: variant.to_owned(),
            body,
        }
    }

    #[test]
    fn a_flat_list_prints_as_its_chars() {
        let list = sym("1+x y");

        assert_eq!(print(&list), "1+x y");
        assert_eq!(parse("1+x y"), list);
    }

    #[test]
    fn resolved_identity_and_variant_round_trip() {
        let node = resolved(
            "physics|vacuum\\permittivity",
            SymbolRole::Constant,
            "greek{small}",
            vec![script(sym("ε"), None, Some(sym("0")))],
        );
        let printed = print(&vec![node.clone()]);

        assert_eq!(parse(&printed), vec![node]);
        assert_eq!(print(&parse(&printed)), printed);
        assert!(printed.starts_with("sym{constant|"));
    }

    #[test]
    fn a_resolved_symbol_after_literal_sym_stays_distinct() {
        let node = resolved("math.pi", SymbolRole::Constant, "greek", sym("π"));
        let mut list = sym("sym");
        list.push(node);
        let printed = print(&list);

        assert_eq!(parse(&printed), list);
        assert!(printed.starts_with("sy\\m{"));
    }

    #[test]
    fn malformed_resolved_headers_parse_as_literal_input() {
        for text in [
            "sym",
            "sym{unknown|id|variant}{x}",
            "sym{constant|missing-variant}{x}",
            "sym{constant|dangling\\",
        ] {
            let parsed = parse(text);
            assert!(!parsed.is_empty(), "{text:?}");
        }
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
    fn a_bar_typed_as_a_symbol_survives_a_print_and_parse_round_trip() {
        let tree = sym("P(A|B)");

        let printed = print(&tree);
        assert_eq!(printed, "P\\(A\\|B\\)");
        assert_eq!(parse(&printed), tree);
    }

    #[test]
    fn each_new_delimiter_pair_round_trips_through_print_and_parse() {
        for &(open, close) in &PAIRS[2..] {
            let tree = vec![group(open, close, sym("x+1"))];
            let printed = print(&tree);

            assert_eq!(parse(&printed), tree);
        }
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
    fn accents_print_with_their_keyword_and_braced_body() {
        for (kind, printed) in [
            (AccentKind::Vector, "vec{x_1}"),
            (AccentKind::Dot, "dot{x_1}"),
            (AccentKind::DoubleDot, "ddot{x_1}"),
            (AccentKind::TripleDot, "dddot{x_1}"),
            (AccentKind::Hat, "hat{x_1}"),
            (AccentKind::Bar, "bar{x_1}"),
        ] {
            let tree = vec![accent(kind, vec![script(sym("x"), None, Some(sym("1")))])];

            assert_eq!(print(&tree), printed);
            assert_eq!(parse(printed), tree);
        }
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
    fn a_contour_integral_round_trips_with_both_limits() {
        let tree = vec![big_op(BigOp::ContourIntegral, sym("C"), sym("R"))];

        assert_eq!(print(&tree), "oint{C}{R}");
        assert_eq!(parse("oint{C}{R}"), tree);
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
        let mut generator = Generator {
            state: 0x5eed_1234_5678_9abc,
            pair_index: 0,
        };

        for _ in 0..500 {
            let tree = generated_list(&mut generator, 4);
            let printed = print(&tree);

            assert_eq!(parse(&printed), tree, "printed: {printed:?}");
            assert_eq!(print(&parse(&printed)), printed, "printed: {printed:?}");
        }

        let mut pair_generator = Generator {
            state: 0,
            pair_index: 0,
        };
        let mut body_generator = Generator {
            state: 0x1234_5678_9abc_def0,
            pair_index: 0,
        };
        let mut generated_pairs = Vec::new();
        for _ in PAIRS {
            let pair = pair_generator.next_pair();
            generated_pairs.push(pair);
            let (open, close) = pair;
            let tree = vec![group(open, close, generated_list(&mut body_generator, 2))];
            let printed = print(&tree);

            assert_eq!(parse(&printed), tree, "printed: {printed:?}");
            assert_eq!(print(&parse(&printed)), printed, "printed: {printed:?}");
        }
        assert_eq!(generated_pairs.as_slice(), PAIRS);
    }

    struct Generator {
        state: u64,
        pair_index: usize,
    }

    impl Generator {
        fn next(&mut self) -> u64 {
            self.state = self
                .state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.state
        }

        fn next_pair(&mut self) -> (char, char) {
            let pair = PAIRS[self.pair_index % PAIRS.len()];
            self.pair_index += 1;
            pair
        }
    }

    fn generated_list(generator: &mut Generator, depth: usize) -> MathList {
        let length = (generator.next() % 5) as usize;
        (0..length)
            .flat_map(|_| {
                let choice = if depth > 0 {
                    generator.next() % 9
                } else {
                    7 + generator.next() % 2
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
                        let (open, close) = generator.next_pair();
                        vec![group(open, close, generated_list(generator, depth - 1))]
                    }
                    3 => vec![sqrt(generated_list(generator, depth - 1))],
                    4 => {
                        let kind = match generator.next() % 5 {
                            0 => BigOp::Sum,
                            1 => BigOp::Prod,
                            2 => BigOp::Integral,
                            3 => BigOp::ContourIntegral,
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
                        let kind = match generator.next() % 6 {
                            0 => AccentKind::Vector,
                            1 => AccentKind::Dot,
                            2 => AccentKind::DoubleDot,
                            3 => AccentKind::TripleDot,
                            4 => AccentKind::Hat,
                            _ => AccentKind::Bar,
                        };
                        vec![accent(kind, generated_list(generator, depth - 1))]
                    }
                    6 => {
                        let role = match generator.next() % 3 {
                            0 => SymbolRole::Variable,
                            1 => SymbolRole::Constant,
                            _ => SymbolRole::Function,
                        };
                        const IDS: &[&str] = &["math.pi", "physics|epsilon", "function\\sin"];
                        const VARIANTS: &[&str] = &["default", "greek{small}", "blackboard"];
                        let id = IDS[(generator.next() % IDS.len() as u64) as usize];
                        let variant = VARIANTS[(generator.next() % VARIANTS.len() as u64) as usize];
                        vec![resolved(
                            id,
                            role,
                            variant,
                            generated_list(generator, depth - 1),
                        )]
                    }
                    7 => {
                        const KEYWORDS: &[&str] = &[
                            "sqrt", "vec", "dot", "ddot", "dddot", "hat", "bar", "sum", "prod",
                            "int", "oint", "lim",
                        ];
                        KEYWORDS[(generator.next() % KEYWORDS.len() as u64) as usize]
                            .chars()
                            .map(MathNode::Sym)
                            .collect()
                    }
                    _ => {
                        const SYMBOLS: &[char] = &[
                            'a', 'Z', '0', '9', '+', '-', '=', '.', ' ', '/', '(', ')', '[', ']',
                            '|', '‖', '⟨', '⟩', '{', '}', '\\', '^', '_',
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
