//! The math tree is presentational-semantic: it knows that a node is a
//! fraction, but nothing about what the expression means. Node kinds are
//! named by meaning, chars are individual atoms in the TeX hlist model, and
//! the cursor is always a plain index into a list. Everything here is pure
//! and testable without a renderer.

/// One atom in a horizontal list.
#[derive(Clone, PartialEq, Debug)]
pub enum MathNode {
    /// One typed character: a digit, letter, operator, comma...
    Sym(char),
    /// A fraction. Slots may be empty; an incomplete expression is legal.
    Frac { num: MathList, den: MathList },
}

pub type MathList = Vec<MathNode>;

/// A named slot of a structural node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Num,
    Den,
}

impl Slot {
    pub fn name(self) -> &'static str {
        match self {
            Self::Num => "num",
            Self::Den => "denom",
        }
    }
}

/// One step down the tree: into node `index` of the current list, through one
/// of its slots.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Step {
    pub index: usize,
    pub slot: Slot,
}

/// Where typing goes inside one math expression: follow `path` from the root
/// list, then sit `index` atoms in, between atoms like a text caret.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MathCursor {
    pub path: Vec<Step>,
    pub index: usize,
}

impl MathNode {
    pub fn slots(&self) -> &'static [Slot] {
        match self {
            Self::Sym(_) => &[],
            Self::Frac { .. } => &[Slot::Num, Slot::Den],
        }
    }

    pub fn is_structural(&self) -> bool {
        !self.slots().is_empty()
    }

    pub fn slot(&self, slot: Slot) -> Option<&MathList> {
        match (self, slot) {
            (Self::Frac { num, .. }, Slot::Num) => Some(num),
            (Self::Frac { den, .. }, Slot::Den) => Some(den),
            (Self::Sym(_), _) => None,
        }
    }

    pub fn slot_mut(&mut self, slot: Slot) -> Option<&mut MathList> {
        match (self, slot) {
            (Self::Frac { num, .. }, Slot::Num) => Some(num),
            (Self::Frac { den, .. }, Slot::Den) => Some(den),
            (Self::Sym(_), _) => None,
        }
    }
}

/// The list `path` names, or `None` when the path no longer matches the tree.
pub fn list_at<'a>(root: &'a MathList, path: &[Step]) -> Option<&'a MathList> {
    let mut list = root;
    for step in path {
        list = list.get(step.index)?.slot(step.slot)?;
    }
    Some(list)
}

pub fn list_at_mut<'a>(root: &'a mut MathList, path: &[Step]) -> Option<&'a mut MathList> {
    let mut list = root;
    for step in path {
        list = list.get_mut(step.index)?.slot_mut(step.slot)?;
    }
    Some(list)
}

/// Clamps a cursor onto `root`, dropping the stale suffix of its path and
/// placing its index within the list reached by the valid prefix.
pub fn clamp(root: &MathList, cursor: &mut MathCursor) {
    let mut list = root;
    let mut valid = 0;
    for step in &cursor.path {
        let Some(next) = list.get(step.index).and_then(|node| node.slot(step.slot)) else {
            break;
        };
        list = next;
        valid += 1;
    }
    cursor.path.truncate(valid);
    cursor.index = cursor.index.min(list.len());
}

/// Inserts one typed character at the cursor.
pub fn insert_char(root: &mut MathList, cursor: &mut MathCursor, c: char) {
    clamp(root, cursor);
    let path = cursor.path.clone();
    let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");
    list.insert(cursor.index, MathNode::Sym(c));
    cursor.index += 1;
}

/// The `/` trigger wraps the preceding operand in a fraction and enters its
/// empty denominator, or its numerator when no operand was present.
pub fn insert_fraction(root: &mut MathList, cursor: &mut MathCursor) {
    clamp(root, cursor);
    let path = cursor.path.clone();
    let index = cursor.index;
    let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");
    let start = if index > 0 && list[index - 1].is_structural() {
        index - 1
    } else {
        let mut start = index;
        while start > 0 {
            let is_operand_char = matches!(
                list[start - 1],
                MathNode::Sym(c) if c.is_alphanumeric() || matches!(c, '.' | '_')
            );
            if !is_operand_char {
                break;
            }
            start -= 1;
        }
        start
    };
    let operand: MathList = list.drain(start..index).collect();
    list.insert(
        start,
        MathNode::Frac {
            num: operand,
            den: Vec::new(),
        },
    );
    cursor.path.push(Step {
        index: start,
        slot: if index == start { Slot::Num } else { Slot::Den },
    });
    cursor.index = 0;
}

/// What backspace did, so the shell can decide whether to exit math.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Removed {
    /// An atom or structure before the cursor was removed or reverted.
    Edited,
    /// Nothing before the cursor in this slot; the cursor climbed out.
    Climbed,
    /// The cursor was at the start of the root list.
    AtStart,
}

pub fn backspace(root: &mut MathList, cursor: &mut MathCursor) -> Removed {
    clamp(root, cursor);
    if cursor.index > 0 {
        let path = cursor.path.clone();
        let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");
        let position = cursor.index - 1;
        match list[position].clone() {
            MathNode::Sym(_) => {
                list.remove(position);
                cursor.index -= 1;
            }
            MathNode::Frac { num, den } => {
                let num_len = num.len();
                list.remove(position);
                list.splice(
                    position..position,
                    num.into_iter()
                        .chain(std::iter::once(MathNode::Sym('/')))
                        .chain(den),
                );
                cursor.index = position + num_len + 1;
            }
        }
        Removed::Edited
    } else if let Some(step) = cursor.path.pop() {
        cursor.index = step.index;
        Removed::Climbed
    } else {
        Removed::AtStart
    }
}

/// Moves right through atoms and into each structural node's slots.
pub fn move_right(root: &MathList, cursor: &mut MathCursor) -> bool {
    clamp(root, cursor);
    let path = cursor.path.clone();
    let list = list_at(root, &path).expect("clamped cursor path must resolve");
    if cursor.index < list.len() {
        let node = &list[cursor.index];
        if let Some(&slot) = node.slots().first() {
            cursor.path.push(Step {
                index: cursor.index,
                slot,
            });
            cursor.index = 0;
        } else {
            cursor.index += 1;
        }
        return true;
    }

    let Some(step) = cursor.path.last().copied() else {
        return false;
    };
    let parent_path = &cursor.path[..cursor.path.len() - 1];
    let parent_list = list_at(root, parent_path).expect("clamped cursor path must resolve");
    let parent = &parent_list[step.index];
    let slots = parent.slots();
    let slot_index = slots
        .iter()
        .position(|candidate| *candidate == step.slot)
        .expect("cursor slot must belong to its parent");
    if let Some(&next) = slots.get(slot_index + 1) {
        cursor
            .path
            .last_mut()
            .expect("cursor path is non-empty")
            .slot = next;
        cursor.index = 0;
    } else {
        let parent_index = step.index;
        cursor.path.pop();
        cursor.index = parent_index + 1;
    }
    true
}

/// Moves left through atoms and into each structural node's slots.
pub fn move_left(root: &MathList, cursor: &mut MathCursor) -> bool {
    clamp(root, cursor);
    if cursor.index > 0 {
        let path = cursor.path.clone();
        let list = list_at(root, &path).expect("clamped cursor path must resolve");
        if let Some(node) = list.get(cursor.index - 1) {
            if let Some(&slot) = node.slots().last() {
                let slot_len = node.slot(slot).expect("listed slot must exist").len();
                cursor.path.push(Step {
                    index: cursor.index - 1,
                    slot,
                });
                cursor.index = slot_len;
            } else {
                cursor.index -= 1;
            }
        }
        return true;
    }

    let Some(step) = cursor.path.last().copied() else {
        return false;
    };
    let parent_path = &cursor.path[..cursor.path.len() - 1];
    let parent_list = list_at(root, parent_path).expect("clamped cursor path must resolve");
    let parent = &parent_list[step.index];
    let slots = parent.slots();
    let slot_index = slots
        .iter()
        .position(|candidate| *candidate == step.slot)
        .expect("cursor slot must belong to its parent");
    if let Some(&previous) = slot_index.checked_sub(1).and_then(|index| slots.get(index)) {
        cursor
            .path
            .last_mut()
            .expect("cursor path is non-empty")
            .slot = previous;
        cursor.index = parent.slot(previous).expect("listed slot must exist").len();
    } else {
        let parent_index = step.index;
        cursor.path.pop();
        cursor.index = parent_index;
    }
    true
}

fn collect_slot_paths(list: &MathList, prefix: &mut Vec<Step>, paths: &mut Vec<Vec<Step>>) {
    for (index, node) in list.iter().enumerate() {
        for &slot in node.slots() {
            prefix.push(Step { index, slot });
            paths.push(prefix.clone());
            collect_slot_paths(
                node.slot(slot).expect("listed slot must exist"),
                prefix,
                paths,
            );
            prefix.pop();
        }
    }
}

fn slot_paths(root: &MathList) -> Vec<Vec<Step>> {
    let mut paths = Vec::new();
    collect_slot_paths(root, &mut Vec::new(), &mut paths);
    paths
}

fn move_to_slot(root: &MathList, cursor: &mut MathCursor, next: bool) -> bool {
    clamp(root, cursor);
    let paths = slot_paths(root);
    if paths.is_empty() {
        return false;
    }
    let empty_exists = paths
        .iter()
        .any(|path| list_at(root, path).is_some_and(Vec::is_empty));
    let candidates: Vec<&Vec<Step>> = paths
        .iter()
        .filter(|path| !empty_exists || list_at(root, path).is_some_and(Vec::is_empty))
        .collect();
    let current = candidates
        .iter()
        .position(|path| path.as_slice() == cursor.path.as_slice());
    let target = if next {
        current.map_or(0, |index| (index + 1) % candidates.len())
    } else {
        current.map_or(candidates.len() - 1, |index| {
            (index + candidates.len() - 1) % candidates.len()
        })
    };
    cursor.path = candidates[target].clone();
    cursor.index = list_at(root, &cursor.path)
        .expect("enumerated slot path must resolve")
        .len();
    true
}

/// Moves to the next empty slot, or the next slot when all slots are filled.
pub fn slot_next(root: &MathList, cursor: &mut MathCursor) -> bool {
    move_to_slot(root, cursor, true)
}

/// Moves to the previous empty slot, or the previous slot when all slots are
/// filled.
pub fn slot_prev(root: &MathList, cursor: &mut MathCursor) -> bool {
    move_to_slot(root, cursor, false)
}

/// Pops one context level, placing the cursor after the structure it leaves.
pub fn pop_level(root: &MathList, cursor: &mut MathCursor) -> bool {
    clamp(root, cursor);
    let Some(step) = cursor.path.pop() else {
        return false;
    };
    cursor.index = step.index + 1;
    true
}

/// Returns the cursor's slot context, outermost first.
pub fn path_names(cursor: &MathCursor) -> Vec<&'static str> {
    cursor.path.iter().map(|step| step.slot.name()).collect()
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

    fn at(index: usize) -> MathCursor {
        MathCursor {
            path: Vec::new(),
            index,
        }
    }

    fn at_path(path: &[(usize, Slot)], index: usize) -> MathCursor {
        MathCursor {
            path: path
                .iter()
                .map(|&(index, slot)| Step { index, slot })
                .collect(),
            index,
        }
    }

    #[test]
    fn typing_builds_sym_atoms() {
        let mut root = Vec::new();
        let mut cursor = MathCursor::default();
        insert_char(&mut root, &mut cursor, '1');
        insert_char(&mut root, &mut cursor, '+');
        insert_char(&mut root, &mut cursor, 'x');

        assert_eq!(root, sym("1+x"));
        assert_eq!(cursor, at(3));
    }

    #[test]
    fn the_slash_trigger_captures_the_preceding_number() {
        let mut root = sym("12");
        let mut cursor = at(2);
        insert_fraction(&mut root, &mut cursor);

        assert_eq!(root, vec![frac(sym("12"), Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Den)], 0));
    }

    #[test]
    fn the_slash_trigger_captures_a_preceding_fraction_for_nesting() {
        let mut root = Vec::new();
        let mut cursor = MathCursor::default();
        insert_char(&mut root, &mut cursor, '1');
        insert_fraction(&mut root, &mut cursor);
        insert_char(&mut root, &mut cursor, '2');
        assert!(move_right(&root, &mut cursor));
        insert_fraction(&mut root, &mut cursor);

        assert_eq!(root, vec![frac(vec![frac(sym("1"), sym("2"))], Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Den)], 0));
    }

    #[test]
    fn the_slash_trigger_stops_at_an_operator() {
        let mut root = sym("1+2");
        let mut cursor = at(3);
        insert_fraction(&mut root, &mut cursor);

        assert_eq!(
            root,
            vec![
                MathNode::Sym('1'),
                MathNode::Sym('+'),
                frac(sym("2"), Vec::new()),
            ]
        );
        assert_eq!(cursor, at_path(&[(2, Slot::Den)], 0));
    }

    #[test]
    fn the_slash_trigger_with_no_operand_starts_in_the_numerator() {
        let mut root = Vec::new();
        let mut cursor = MathCursor::default();
        insert_fraction(&mut root, &mut cursor);

        assert_eq!(root, vec![frac(Vec::new(), Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Num)], 0));
    }

    #[test]
    fn backspace_removes_a_sym() {
        let mut root = sym("ab");
        let mut cursor = at(2);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("a"));
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn backspace_reverts_a_fraction_to_its_literal_atoms() {
        let mut root = vec![frac(sym("12"), sym("34"))];
        let mut cursor = at(1);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("12/34"));
        assert_eq!(cursor, at(3));
    }

    #[test]
    fn backspace_at_a_slot_start_climbs_out_without_deleting() {
        let mut root = vec![frac(sym("a"), Vec::new())];
        let original = root.clone();
        let mut cursor = at_path(&[(0, Slot::Den)], 0);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Climbed);
        assert_eq!(root, original);
        assert_eq!(cursor, at(0));
    }

    #[test]
    fn backspace_at_the_root_start_reports_at_start() {
        let mut root = sym("a");
        let mut cursor = MathCursor::default();

        assert_eq!(backspace(&mut root, &mut cursor), Removed::AtStart);
        assert_eq!(root, sym("a"));
        assert_eq!(cursor, MathCursor::default());
    }

    #[test]
    fn arrows_walk_through_every_slot() {
        let root = vec![
            MathNode::Sym('a'),
            frac(sym("b"), sym("c")),
            MathNode::Sym('d'),
        ];
        let expected = [
            at(1),
            at_path(&[(1, Slot::Num)], 0),
            at_path(&[(1, Slot::Num)], 1),
            at_path(&[(1, Slot::Den)], 0),
            at_path(&[(1, Slot::Den)], 1),
            at(2),
            at(3),
        ];
        let mut cursor = MathCursor::default();
        for expected in expected {
            assert!(move_right(&root, &mut cursor));
            assert_eq!(cursor, expected);
        }
        assert!(!move_right(&root, &mut cursor));

        for expected in [
            at(2),
            at_path(&[(1, Slot::Den)], 1),
            at_path(&[(1, Slot::Den)], 0),
            at_path(&[(1, Slot::Num)], 1),
            at_path(&[(1, Slot::Num)], 0),
            at(1),
            at(0),
        ] {
            assert!(move_left(&root, &mut cursor));
            assert_eq!(cursor, expected);
        }
        assert!(!move_left(&root, &mut cursor));
    }

    #[test]
    fn tab_visits_empty_slots_first() {
        let root = vec![frac(sym("x"), vec![frac(Vec::new(), Vec::new())])];
        let mut cursor = MathCursor::default();

        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Den), (0, Slot::Num)], 0));
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Den), (0, Slot::Den)], 0));
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Den), (0, Slot::Num)], 0));
    }

    #[test]
    fn tab_walks_all_slots_when_none_are_empty() {
        let root = vec![frac(vec![frac(sym("a"), sym("b"))], sym("c"))];
        let mut cursor = MathCursor::default();
        let paths = [
            at_path(&[(0, Slot::Num)], 1),
            at_path(&[(0, Slot::Num), (0, Slot::Num)], 1),
            at_path(&[(0, Slot::Num), (0, Slot::Den)], 1),
            at_path(&[(0, Slot::Den)], 1),
        ];
        for expected in &paths {
            assert!(slot_next(&root, &mut cursor));
            assert_eq!(&cursor, expected);
        }

        cursor = MathCursor::default();
        for expected in paths.iter().rev() {
            assert!(slot_prev(&root, &mut cursor));
            assert_eq!(&cursor, expected);
        }
    }

    #[test]
    fn esc_pops_one_level_then_reports_root() {
        let root = vec![frac(Vec::new(), vec![frac(Vec::new(), Vec::new())])];
        let mut cursor = at_path(&[(0, Slot::Den), (0, Slot::Num)], 0);

        assert!(pop_level(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Den)], 1));
        assert!(pop_level(&root, &mut cursor));
        assert_eq!(cursor, at(1));
        assert!(!pop_level(&root, &mut cursor));
    }

    #[test]
    fn a_stale_cursor_is_clamped_not_trusted() {
        let root = sym("x");
        let mut cursor = at_path(&[(0, Slot::Den)], 99);

        clamp(&root, &mut cursor);
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn path_names_spell_the_status_line() {
        let cursor = at_path(&[(0, Slot::Den), (0, Slot::Den)], 0);

        assert_eq!(path_names(&cursor), vec!["denom", "denom"]);
    }
}
