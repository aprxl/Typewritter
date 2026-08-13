//! Canonical linear notation for math trees.
//!
//! Deterministic printing is a correctness requirement: auto-commit compares
//! serialized note state, so non-deterministic output creates phantom commits.
//! The notation has two round-trip properties: `print` followed by `parse` is
//! identity on trees, and `parse` followed by `print` is byte-identity on
//! strings produced by `print`. Structural characters in `Sym` nodes are
//! escaped with a backslash; a trailing backslash is the literal backslash.

use super::math::{MathList, MathNode};

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
        if index > 0 && matches!(node, MathNode::Frac { .. }) {
            // A later fraction with an empty numerator would capture the preceding node.
            output.push('(');
            print_node(node, output);
            output.push(')');
        } else {
            print_node(node, output);
        }
    }
}

fn print_node(node: &MathNode, output: &mut String) {
    match node {
        MathNode::Sym(c) => {
            if matches!(c, '/' | '(' | ')' | '\\') {
                output.push('\\');
            }
            output.push(*c);
        }
        MathNode::Frac { num, den } => {
            print_operand(num, false, output);
            output.push('/');
            print_operand(den, true, output);
        }
    }
}

fn print_operand(list: &MathList, denominator: bool, output: &mut String) {
    let bare = list.len() == 1
        // Left associativity reconstructs a single fraction numerator. A
        // denominator fraction needs parentheses or it becomes that same
        // left-associated numerator.
        && !(denominator && matches!(list.first(), Some(MathNode::Frac { .. })));
    if bare {
        print_node(&list[0], output);
    } else {
        output.push('(');
        print_list(list, output);
        output.push(')');
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
    .parse_list(false)
}

struct Parser {
    chars: Vec<char>,
    position: usize,
}

impl Parser {
    fn parse_list(&mut self, grouped: bool) -> MathList {
        let mut items = Vec::new();

        while let Some(&current) = self.chars.get(self.position) {
            if grouped && current == ')' {
                self.position += 1;
                break;
            }

            if current == '/' {
                self.position += 1;
                let num = items.pop().unwrap_or_default();
                let den = if self.starts_operand(grouped) {
                    self.parse_item().unwrap_or_default()
                } else {
                    Vec::new()
                };
                items.push(vec![MathNode::Frac { num, den }]);
            } else if let Some(item) = self.parse_item() {
                items.push(item);
            }
        }

        items.into_iter().flatten().collect()
    }

    fn starts_operand(&self, grouped: bool) -> bool {
        match self.chars.get(self.position) {
            None | Some('/') => false,
            Some(')') if grouped => false,
            _ => true,
        }
    }

    fn parse_item(&mut self) -> Option<MathList> {
        let current = *self.chars.get(self.position)?;
        match current {
            '/' => None,
            '(' => {
                self.position += 1;
                Some(self.parse_list(true))
            }
            '\\' => {
                self.position += 1;
                let escaped = self.chars.get(self.position).copied().unwrap_or('\\');
                if self.position < self.chars.len() {
                    self.position += 1;
                }
                Some(vec![MathNode::Sym(escaped)])
            }
            ')' => {
                self.position += 1;
                Some(vec![MathNode::Sym(')')])
            }
            c => {
                self.position += 1;
                Some(vec![MathNode::Sym(c)])
            }
        }
    }
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

    #[test]
    fn a_flat_list_prints_as_its_chars() {
        let list = sym("1+x y");

        assert_eq!(print(&list), "1+x y");
        assert_eq!(parse("1+x y"), list);
    }

    #[test]
    fn a_fraction_prints_operands_bare_only_when_single() {
        let fraction = frac(sym("R_2"), sym("R_1"));

        assert_eq!(print(&vec![fraction.clone()]), "(R_2)/(R_1)");
        assert_eq!(parse("(R_2)/(R_1)"), vec![fraction]);
        assert_eq!(print(&vec![frac(sym("a"), sym("b"))]), "a/b");
    }

    #[test]
    fn empty_slots_print_as_empty_parens() {
        let list = vec![frac(Vec::new(), Vec::new())];

        assert_eq!(print(&list), "()/()");
        assert_eq!(parse("()/()"), list);
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
    fn a_denominator_fraction_needs_its_parens() {
        let left_associative = vec![frac(vec![frac(sym("1"), sym("2"))], sym("3"))];
        let denominator_fraction = vec![frac(sym("1"), vec![frac(sym("2"), sym("3"))])];

        assert_eq!(print(&left_associative), "1/2/3");
        assert_eq!(print(&denominator_fraction), "1/(2/3)");
        assert_ne!(print(&left_associative), print(&denominator_fraction));
        assert_eq!(parse(&print(&left_associative)), left_associative);
        assert_eq!(parse(&print(&denominator_fraction)), denominator_fraction);
    }

    #[test]
    fn an_empty_numerator_after_an_atom_keeps_its_parens() {
        let list = vec![MathNode::Sym('a'), frac(Vec::new(), sym("b"))];

        let printed = print(&list);
        assert_eq!(printed, "a(()/b)");
        assert_eq!(parse(&printed), list);
    }

    #[test]
    fn unmatched_parens_do_not_panic() {
        let parsed = parse(")((a");

        assert_eq!(parsed, vec![MathNode::Sym(')'), MathNode::Sym('a')]);
        assert_eq!(print(&parsed), "\\)a");
        assert_eq!(parse(&print(&parsed)), parsed);
    }

    #[test]
    fn hand_typed_spaces_survive() {
        let parsed = parse("a / b");
        let printed = print(&parsed);

        assert_eq!(printed, "a( / )b");
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
            .map(|_| {
                if depth > 0 && generator.next().is_multiple_of(4) {
                    frac(
                        generated_list(generator, depth - 1),
                        generated_list(generator, depth - 1),
                    )
                } else {
                    const SYMBOLS: &[char] = &[
                        'a', 'Z', '0', '9', '+', '-', '=', '.', ' ', '/', '(', ')', '\\',
                    ];
                    MathNode::Sym(SYMBOLS[(generator.next() % SYMBOLS.len() as u64) as usize])
                }
            })
            .collect()
    }
}
