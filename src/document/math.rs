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
    /// A base with scripts attached. `None` means that script was not asked for;
    /// `Some` (possibly empty) means the slot exists and can be typed into.
    Script {
        base: MathList,
        sup: Option<MathList>,
        sub: Option<MathList>,
    },
    /// A bracketed group. The delimiters are stored as the characters they
    /// are drawn as, so one node covers every pair without a variant each.
    Group {
        open: char,
        close: char,
        body: MathList,
    },
    /// A radical. One slot today; an index is an additive change to this node.
    Sqrt { body: MathList },
    /// A notation accent drawn above its body.
    Accent { kind: AccentKind, body: MathList },
    /// A large operator carrying its always-present limit slots.
    BigOp {
        kind: BigOp,
        lower: MathList,
        upper: MathList,
    },
}

pub type MathList = Vec<MathNode>;

/// Which accent is drawn above a node's body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccentKind {
    Vector,
    Dot,
    DoubleDot,
    TripleDot,
}

impl AccentKind {
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Vector => "vec",
            Self::Dot => "dot",
            Self::DoubleDot => "ddot",
            Self::TripleDot => "dddot",
        }
    }
}

/// Which large operator, named by what it means rather than by its glyph.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BigOp {
    Sum,
    Prod,
    Integral,
    ContourIntegral,
    Limit,
}

impl BigOp {
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Prod => "prod",
            Self::Integral => "int",
            Self::ContourIntegral => "oint",
            Self::Limit => "lim",
        }
    }
}

fn empty_sqrt() -> MathNode {
    MathNode::Sqrt { body: Vec::new() }
}

fn empty_vec() -> MathNode {
    empty_accent(AccentKind::Vector)
}

fn empty_dot() -> MathNode {
    empty_accent(AccentKind::Dot)
}

fn empty_ddot() -> MathNode {
    empty_accent(AccentKind::DoubleDot)
}

fn empty_dddot() -> MathNode {
    empty_accent(AccentKind::TripleDot)
}

fn empty_accent(kind: AccentKind) -> MathNode {
    MathNode::Accent {
        kind,
        body: Vec::new(),
    }
}

fn empty_sum() -> MathNode {
    MathNode::BigOp {
        kind: BigOp::Sum,
        lower: Vec::new(),
        upper: Vec::new(),
    }
}

fn empty_prod() -> MathNode {
    MathNode::BigOp {
        kind: BigOp::Prod,
        lower: Vec::new(),
        upper: Vec::new(),
    }
}

fn empty_integral() -> MathNode {
    MathNode::BigOp {
        kind: BigOp::Integral,
        lower: Vec::new(),
        upper: Vec::new(),
    }
}

fn empty_contour_integral() -> MathNode {
    MathNode::BigOp {
        kind: BigOp::ContourIntegral,
        lower: Vec::new(),
        upper: Vec::new(),
    }
}

fn empty_limit() -> MathNode {
    MathNode::BigOp {
        kind: BigOp::Limit,
        lower: Vec::new(),
        upper: Vec::new(),
    }
}

/// Words that become structures when a space is typed after them.
pub(crate) type Word = (&'static str, fn() -> MathNode);

pub(crate) const WORDS: &[Word] = &[
    ("sqrt", empty_sqrt),
    (AccentKind::Vector.keyword(), empty_vec),
    (AccentKind::Dot.keyword(), empty_dot),
    (AccentKind::DoubleDot.keyword(), empty_ddot),
    (AccentKind::TripleDot.keyword(), empty_dddot),
    (BigOp::Sum.keyword(), empty_sum),
    (BigOp::Prod.keyword(), empty_prod),
    (BigOp::Integral.keyword(), empty_integral),
    (BigOp::ContourIntegral.keyword(), empty_contour_integral),
    (BigOp::Limit.keyword(), empty_limit),
];

/// A structure the completion palette offers by name, with the preview its
/// card row shows.
pub struct Structure {
    pub name: &'static str,
    pub preview: &'static str,
}

/// The palette's structure offers: the space-trigger words plus the ones
/// only the palette reaches, in offer order.
pub const STRUCTURES: &[Structure] = &[
    Structure {
        name: "frac",
        preview: "a/b",
    },
    Structure {
        name: "sqrt",
        preview: "√",
    },
    Structure {
        name: "vec",
        preview: "x⃗",
    },
    Structure {
        name: "dot",
        preview: "ẋ",
    },
    Structure {
        name: "ddot",
        preview: "ẍ",
    },
    Structure {
        name: "dddot",
        preview: "x⃛",
    },
    Structure {
        name: "sum",
        preview: "∑",
    },
    Structure {
        name: "prod",
        preview: "∏",
    },
    Structure {
        name: "int",
        preview: "∫",
    },
    Structure {
        name: "oint",
        preview: "∮",
    },
    Structure {
        name: "lim",
        preview: "lim",
    },
    Structure {
        name: "sup",
        preview: "x^2",
    },
    Structure {
        name: "sub",
        preview: "x_2",
    },
    Structure {
        name: "paren",
        preview: "( )",
    },
    Structure {
        name: "brack",
        preview: "[ ]",
    },
];

/// One completion offered inside an expression.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Completion {
    /// A symbol from the symbol table, replacing the typed word.
    Symbol {
        name: &'static str,
        group: &'static str,
        glyph: char,
    },
    /// A structure built in place of the typed word.
    Structure {
        name: &'static str,
        preview: &'static str,
    },
}

/// Everything that completes `word`, symbols first and structures second,
/// each with an exact match before its prefixes — the same ordering rule
/// as the symbol table's own matching.
pub fn completions(word: &str) -> Vec<Completion> {
    let mut out: Vec<Completion> = crate::document::math_symbols::matching(word)
        .iter()
        .map(|symbol| Completion::Symbol {
            name: symbol.name,
            group: symbol.group,
            glyph: symbol.glyph,
        })
        .collect();
    for structure in STRUCTURES.iter().filter(|structure| structure.name == word) {
        out.push(Completion::Structure {
            name: structure.name,
            preview: structure.preview,
        });
    }
    for structure in STRUCTURES
        .iter()
        .filter(|structure| structure.name != word && structure.name.starts_with(word))
    {
        out.push(Completion::Structure {
            name: structure.name,
            preview: structure.preview,
        });
    }
    out
}

/// A named slot of a structural node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Base,
    Num,
    Den,
    Sup,
    Sub,
    Body,
    Lower,
    Upper,
}

impl Slot {
    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Num => "num",
            Self::Den => "denom",
            Self::Sup => "sup",
            Self::Sub => "sub",
            Self::Body => "body",
            Self::Lower => "lower",
            Self::Upper => "upper",
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
    pub fn slots(&self) -> Vec<Slot> {
        match self {
            Self::Sym(_) => Vec::new(),
            Self::Frac { .. } => vec![Slot::Num, Slot::Den],
            Self::Script { sup, sub, .. } => {
                let mut slots = vec![Slot::Base];
                if sup.is_some() {
                    slots.push(Slot::Sup);
                }
                if sub.is_some() {
                    slots.push(Slot::Sub);
                }
                slots
            }
            Self::Group { .. } => vec![Slot::Body],
            Self::Sqrt { .. } => vec![Slot::Body],
            Self::Accent { .. } => vec![Slot::Body],
            Self::BigOp { kind, .. } => {
                if *kind == BigOp::Limit {
                    vec![Slot::Lower]
                } else {
                    vec![Slot::Lower, Slot::Upper]
                }
            }
        }
    }

    pub fn is_structural(&self) -> bool {
        !self.slots().is_empty()
    }

    pub fn slot(&self, slot: Slot) -> Option<&MathList> {
        match (self, slot) {
            (Self::Frac { num, .. }, Slot::Num) => Some(num),
            (Self::Frac { den, .. }, Slot::Den) => Some(den),
            (Self::Script { base, .. }, Slot::Base) => Some(base),
            (Self::Script { sup: Some(sup), .. }, Slot::Sup) => Some(sup),
            (Self::Script { sub: Some(sub), .. }, Slot::Sub) => Some(sub),
            (Self::Group { body, .. }, Slot::Body) => Some(body),
            (Self::Sqrt { body }, Slot::Body) => Some(body),
            (Self::Accent { body, .. }, Slot::Body) => Some(body),
            (Self::BigOp { lower, .. }, Slot::Lower) => Some(lower),
            (Self::BigOp { kind, upper, .. }, Slot::Upper) if *kind != BigOp::Limit => Some(upper),
            (Self::Sym(_), _) => None,
            (Self::Frac { .. }, _)
            | (Self::Script { .. }, _)
            | (Self::Group { .. }, _)
            | (Self::Sqrt { .. }, _)
            | (Self::Accent { .. }, _)
            | (Self::BigOp { .. }, _) => None,
        }
    }

    pub fn slot_mut(&mut self, slot: Slot) -> Option<&mut MathList> {
        match (self, slot) {
            (Self::Frac { num, .. }, Slot::Num) => Some(num),
            (Self::Frac { den, .. }, Slot::Den) => Some(den),
            (Self::Script { base, .. }, Slot::Base) => Some(base),
            (Self::Script { sup: Some(sup), .. }, Slot::Sup) => Some(sup),
            (Self::Script { sub: Some(sub), .. }, Slot::Sub) => Some(sub),
            (Self::Group { body, .. }, Slot::Body) => Some(body),
            (Self::Sqrt { body }, Slot::Body) => Some(body),
            (Self::Accent { body, .. }, Slot::Body) => Some(body),
            (Self::BigOp { lower, .. }, Slot::Lower) => Some(lower),
            (Self::BigOp { kind, upper, .. }, Slot::Upper) if *kind != BigOp::Limit => Some(upper),
            (Self::Sym(_), _) => None,
            (Self::Frac { .. }, _)
            | (Self::Script { .. }, _)
            | (Self::Group { .. }, _)
            | (Self::Sqrt { .. }, _)
            | (Self::Accent { .. }, _)
            | (Self::BigOp { .. }, _) => None,
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

/// A space was typed: replace a standalone trigger word and enter its first slot.
pub fn insert_word(root: &mut MathList, cursor: &mut MathCursor) -> bool {
    clamp(root, cursor);
    let path = cursor.path.clone();
    let index = cursor.index;
    let Some((_, build)) = list_at(root, &path).and_then(|list| {
        if index == 0 || !matches!(list[index - 1], MathNode::Sym(c) if c.is_alphabetic()) {
            return None;
        }
        let mut start = index;
        while start > 0 && matches!(list[start - 1], MathNode::Sym(c) if c.is_alphabetic()) {
            start -= 1;
        }
        if start > 0
            && matches!(list[start - 1], MathNode::Sym(c) if c.is_alphanumeric() || c == '_')
        {
            return None;
        }
        WORDS
            .iter()
            .find(|(word, _)| {
                word.chars().count() == index - start
                    && list[start..index]
                        .iter()
                        .zip(word.chars())
                        .all(|(node, c)| matches!(node, MathNode::Sym(atom) if *atom == c))
            })
            .map(|(_, build)| (start, *build))
    }) else {
        return false;
    };

    take_word(root, cursor);
    insert_node(root, cursor, build());
    true
}

/// Inserts `node` at the cursor and enters its first slot — the shared tail
/// of the word trigger and a structure completion.
fn insert_node(root: &mut MathList, cursor: &mut MathCursor, node: MathNode) {
    let path = cursor.path.clone();
    let index = cursor.index;
    let slot = node.slots()[0];
    let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");
    list.insert(index, node);
    cursor.path.push(Step { index, slot });
    cursor.index = 0;
}

/// Builds the palette structure named `name` in place of the word before the
/// cursor, entering its first slot. `false` when `name` is not one of
/// [`STRUCTURES`].
pub fn insert_structure(root: &mut MathList, cursor: &mut MathCursor, name: &str) -> bool {
    if !STRUCTURES.iter().any(|structure| structure.name == name) {
        return false;
    }
    take_word(root, cursor);
    match name {
        "frac" => insert_fraction(root, cursor),
        "sup" => insert_script(root, cursor, Slot::Sup),
        "sub" => insert_script(root, cursor, Slot::Sub),
        "paren" => {
            insert_group(root, cursor, '(');
        }
        "brack" => {
            insert_group(root, cursor, '[');
        }
        "sqrt" => insert_node(root, cursor, empty_sqrt()),
        "vec" => insert_node(root, cursor, empty_vec()),
        "dot" => insert_node(root, cursor, empty_dot()),
        "ddot" => insert_node(root, cursor, empty_ddot()),
        "dddot" => insert_node(root, cursor, empty_dddot()),
        "sum" => insert_node(root, cursor, empty_sum()),
        "prod" => insert_node(root, cursor, empty_prod()),
        "int" => insert_node(root, cursor, empty_integral()),
        "oint" => insert_node(root, cursor, empty_contour_integral()),
        "lim" => insert_node(root, cursor, empty_limit()),
        _ => unreachable!("checked against STRUCTURES"),
    }
    true
}

fn word_range(list: &MathList, index: usize) -> Option<(usize, usize)> {
    if index == 0 || !matches!(list[index - 1], MathNode::Sym(c) if c.is_alphabetic()) {
        return None;
    }
    let mut start = index;
    while start > 0 && matches!(list[start - 1], MathNode::Sym(c) if c.is_alphabetic()) {
        start -= 1;
    }
    Some((start, index))
}

/// The run of letters immediately before the cursor, which is what a
/// completion is being typed into. `None` when the cursor is not after
/// one. Read from the tree rather than from what was typed, so it stays
/// right through backspace, arrow moves and a click elsewhere without
/// any state of its own to fall out of step.
pub fn word_before(root: &MathList, cursor: &MathCursor) -> Option<String> {
    let mut cursor = cursor.clone();
    clamp(root, &mut cursor);
    let list = list_at(root, &cursor.path)?;
    let (start, end) = word_range(list, cursor.index)?;
    Some(
        list[start..end]
            .iter()
            .map(|node| match node {
                MathNode::Sym(c) => *c,
                _ => unreachable!("word range contains only letter atoms"),
            })
            .collect(),
    )
}

/// Replaces the word before the cursor with `glyph`. Used when a
/// completion is accepted; the word is what `word_before` reported.
pub fn accept_symbol(root: &mut MathList, cursor: &mut MathCursor, glyph: char) {
    clamp(root, cursor);
    let path = cursor.path.clone();
    let index = cursor.index;
    let Some((start, end)) = list_at(root, &path).and_then(|list| word_range(list, index)) else {
        return;
    };
    let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");
    list.splice(start..end, std::iter::once(MathNode::Sym(glyph)));
    cursor.index = start + 1;
}

/// Removes the word before the cursor without inserting anything, so a
/// caller can follow it with a structure trigger.
pub fn take_word(root: &mut MathList, cursor: &mut MathCursor) {
    clamp(root, cursor);
    let path = cursor.path.clone();
    let index = cursor.index;
    let Some((start, end)) = list_at(root, &path).and_then(|list| word_range(list, index)) else {
        return;
    };
    let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");
    list.drain(start..end);
    cursor.index = start;
}

/// The delimiter pairs a typed opener produces. Braces are deliberately
/// absent: `{` and `}` are the notation's invisible grouping (see
/// `math_notation`), and the brace people actually write in notation is the
/// one a `cases` block draws, which is a structure of its own.
pub const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']')];

/// Opens a bracket group at the cursor and enters it, if `c` is an opener.
/// Auto-paired: the closer is part of the node, so it can never be left
/// unmatched by typing.
pub fn insert_group(root: &mut MathList, cursor: &mut MathCursor, c: char) -> bool {
    let Some(&(_, close)) = PAIRS.iter().find(|&&(open, _)| open == c) else {
        return false;
    };
    clamp(root, cursor);
    let path = cursor.path.clone();
    let index = cursor.index;
    let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");

    // Unlike `/` and `^`, a bracket wraps what comes next, not what came before.
    list.insert(
        index,
        MathNode::Group {
            open: c,
            close,
            body: Vec::new(),
        },
    );
    cursor.path.push(Step {
        index,
        slot: Slot::Body,
    });
    cursor.index = 0;
    true
}

/// Steps out of the innermost group when `c` closes it — typing the closing
/// bracket means "I am done in here", not "insert a character", since the
/// closer already exists. `false` when the cursor is not in a group `c` closes,
/// and the caller should type it literally.
pub fn close_group(root: &mut MathList, cursor: &mut MathCursor, c: char) -> bool {
    clamp(root, cursor);
    for path_index in (0..cursor.path.len()).rev() {
        let step = cursor.path[path_index];
        if step.slot != Slot::Body {
            continue;
        }
        let parent_path = &cursor.path[..path_index];
        let Some(MathNode::Group { close, .. }) =
            list_at(root, parent_path).and_then(|list| list.get(step.index))
        else {
            continue;
        };
        if *close == c {
            cursor.path.truncate(path_index);
            cursor.index = step.index + 1;
            return true;
        }
    }
    false
}

fn capture_operand(list: &mut MathList, index: usize) -> (usize, MathList) {
    let start = if index > 0 && list[index - 1].is_structural() {
        index - 1
    } else {
        let mut start = index;
        while start > 0 {
            let is_operand_char = matches!(
                list[start - 1],
                // `_` is a trigger now, so never swallow it into an identifier.
                MathNode::Sym(c) if c.is_alphanumeric() || c == '.'
            );
            if !is_operand_char {
                break;
            }
            start -= 1;
        }
        start
    };
    (start, list.drain(start..index).collect())
}

/// The `/` trigger wraps the preceding operand in a fraction and enters its
/// empty denominator, or its numerator when no operand was present.
pub fn insert_fraction(root: &mut MathList, cursor: &mut MathCursor) {
    clamp(root, cursor);
    let path = cursor.path.clone();
    let index = cursor.index;
    let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");
    let (start, operand) = capture_operand(list, index);
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

/// The `^` and `_` triggers attach a script to the preceding operand.
pub fn insert_script(root: &mut MathList, cursor: &mut MathCursor, which: Slot) {
    if !matches!(which, Slot::Sup | Slot::Sub) {
        return;
    }
    clamp(root, cursor);
    let path = cursor.path.clone();
    let index = cursor.index;

    // 1. Inside one script slot, fill the parent's other empty slot. A script
    // nested directly inside another script can no longer be typed in one
    // flow; grouping brackets from the next structure are required for that.
    if let Some(step) = path.last()
        && matches!(step.slot, Slot::Sup | Slot::Sub)
        && step.slot != which
    {
        let parent_path = &path[..path.len() - 1];
        let parent_list = list_at_mut(root, parent_path).expect("clamped cursor path must resolve");
        if let MathNode::Script { sup, sub, .. } = &mut parent_list[step.index] {
            let other = match which {
                Slot::Sup => sup,
                Slot::Sub => sub,
                Slot::Base | Slot::Num | Slot::Den | Slot::Body | Slot::Lower | Slot::Upper => {
                    unreachable!()
                }
            };
            if other.is_none() {
                *other = Some(Vec::new());
                cursor
                    .path
                    .last_mut()
                    .expect("cursor path is non-empty")
                    .slot = which;
                cursor.index = 0;
                return;
            }
        }
    }

    let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");

    // 2. Immediately after a script, enter its missing slot.
    if index > 0
        && let MathNode::Script { sup, sub, .. } = &mut list[index - 1]
    {
        let target = match which {
            Slot::Sup => sup,
            Slot::Sub => sub,
            Slot::Base | Slot::Num | Slot::Den | Slot::Body | Slot::Lower | Slot::Upper => {
                unreachable!()
            }
        };
        if target.is_none() {
            *target = Some(Vec::new());
            cursor.path.push(Step {
                index: index - 1,
                slot: which,
            });
            cursor.index = 0;
            return;
        }
    }

    // 3. Otherwise, capture the preceding operand and wrap it.
    let (start, operand) = capture_operand(list, index);
    let empty_operand = operand.is_empty();
    list.insert(
        start,
        MathNode::Script {
            base: operand,
            sup: (which == Slot::Sup).then(Vec::new),
            sub: (which == Slot::Sub).then(Vec::new),
        },
    );
    cursor.path.push(Step {
        index: start,
        slot: if empty_operand { Slot::Base } else { which },
    });
    cursor.index = 0;
}

/// Replaces one structural node with the literal atoms that trigger it and
/// returns the cursor position immediately after that trigger.
fn flatten_structure(list: &mut MathList, position: usize) -> Option<usize> {
    let (replacement, trigger_offset) = match list.get(position)?.clone() {
        MathNode::Sym(_) => return None,
        MathNode::Frac { num, den } => {
            let trigger_offset = num.len() + 1;
            let mut replacement = num;
            replacement.push(MathNode::Sym('/'));
            replacement.extend(den);
            (replacement, trigger_offset)
        }
        MathNode::Script { base, sup, sub } => {
            let trigger_offset = base.len() + usize::from(sup.is_some() || sub.is_some());
            let mut replacement = base;
            if let Some(sup) = sup {
                replacement.push(MathNode::Sym('^'));
                replacement.extend(sup);
            }
            if let Some(sub) = sub {
                replacement.push(MathNode::Sym('_'));
                replacement.extend(sub);
            }
            (replacement, trigger_offset)
        }
        MathNode::Group { open, close, body } => {
            let mut replacement = vec![MathNode::Sym(open)];
            replacement.extend(body);
            replacement.push(MathNode::Sym(close));
            (replacement, 1)
        }
        MathNode::Sqrt { body } => {
            let keyword = "sqrt";
            let mut replacement = keyword.chars().map(MathNode::Sym).collect::<MathList>();
            replacement.extend(body);
            (replacement, keyword.chars().count())
        }
        MathNode::Accent { kind, body } => {
            let keyword = kind.keyword();
            let mut replacement = keyword.chars().map(MathNode::Sym).collect::<MathList>();
            replacement.extend(body);
            (replacement, keyword.chars().count())
        }
        MathNode::BigOp { kind, lower, upper } => {
            let keyword = kind.keyword();
            let mut replacement = keyword.chars().map(MathNode::Sym).collect::<MathList>();
            replacement.push(MathNode::Sym('_'));
            replacement.extend(lower);
            if kind != BigOp::Limit {
                replacement.push(MathNode::Sym('^'));
                replacement.extend(upper);
            }
            (replacement, keyword.chars().count())
        }
    };
    list.splice(position..=position, replacement);
    Some(position + trigger_offset)
}

/// What deletion did, so the shell can decide whether to exit math.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Removed {
    /// An atom or structure before the cursor was removed or reverted.
    Edited,
    /// Nothing before the cursor in this slot; the cursor climbed out.
    Climbed,
    /// The cursor was at the start of the root list.
    AtStart,
    /// The cursor was at the end of the root list.
    AtEnd,
}

pub fn backspace(root: &mut MathList, cursor: &mut MathCursor) -> Removed {
    clamp(root, cursor);
    if cursor.index > 0 {
        let path = cursor.path.clone();
        let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");
        let position = cursor.index - 1;
        if let Some(landing) = flatten_structure(list, position) {
            cursor.index = landing;
        } else {
            list.remove(position);
            cursor.index -= 1;
        }
        Removed::Edited
    } else if !cursor.path.is_empty()
        && list_at(root, &cursor.path)
            .expect("clamped cursor path must resolve")
            .is_empty()
    {
        let step = *cursor.path.last().expect("cursor path is non-empty");
        let parent_path = cursor.path[..cursor.path.len() - 1].to_vec();
        let parent = list_at_mut(root, &parent_path).expect("clamped cursor path must resolve");
        cursor.index = flatten_structure(parent, step.index)
            .expect("a cursor slot must belong to a structural node");
        cursor.path.pop();
        Removed::Edited
    } else if let Some(step) = cursor.path.pop() {
        cursor.index = step.index;
        Removed::Climbed
    } else {
        Removed::AtStart
    }
}

/// Deletes the next atom, crossing slot boundaries in document order.
pub fn delete_forward(root: &mut MathList, cursor: &mut MathCursor) -> Removed {
    clamp(root, cursor);
    loop {
        let path = cursor.path.clone();
        let list_len = list_at(root, &path)
            .expect("clamped cursor path must resolve")
            .len();
        if cursor.index < list_len {
            let list = list_at_mut(root, &path).expect("clamped cursor path must resolve");
            if let Some(landing) = flatten_structure(list, cursor.index) {
                cursor.index = landing;
            } else {
                list.remove(cursor.index);
            }
            return Removed::Edited;
        }
        if !move_right(root, cursor) {
            return Removed::AtEnd;
        }
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

/// One Tab stop inside an expression: a structural slot (cursor at its end)
/// or an exit (cursor right after a structural node, back in its parent
/// list). Every template contributes both, in document order, so Tab walks
/// its slots and then out of it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Position {
    path: Vec<Step>,
    index: usize,
    exit: bool,
}

/// The Tab order of every slot and exit in the expression. Content first,
/// then a slot's own stop, then the node's exit.
fn tab_positions(root: &MathList) -> Vec<Position> {
    fn collect(list: &MathList, path: &[Step], out: &mut Vec<Position>) {
        for (index, node) in list.iter().enumerate() {
            for slot in node.slots() {
                let slot_list = node.slot(slot).expect("listed slot must exist");
                let mut child = path.to_vec();
                child.push(Step { index, slot });
                collect(slot_list, &child, out);
                out.push(Position {
                    path: child,
                    index: slot_list.len(),
                    exit: false,
                });
            }
            if node.is_structural() {
                // A slot stop and a nested exit can be the same spot (a
                // group is the only child of a slot); keep the first.
                let exit = Position {
                    path: path.to_vec(),
                    index: index + 1,
                    exit: true,
                };
                if !out
                    .iter()
                    .any(|p| p.path == exit.path && p.index == exit.index)
                {
                    out.push(exit);
                }
            }
        }
    }
    let mut out = Vec::new();
    collect(root, &[], &mut out);
    out
}

/// Document order of two cursor positions. Slot ranks are only compared
/// between slots of one node, so their order just has to match `slots()`.
fn slot_rank(slot: Slot) -> u8 {
    match slot {
        Slot::Num => 0,
        Slot::Den => 1,
        Slot::Base => 2,
        Slot::Sup => 3,
        Slot::Sub => 4,
        Slot::Body => 5,
        Slot::Lower => 6,
        Slot::Upper => 7,
    }
}

/// Whether the position at `a` comes before the one at `b`, in reading
/// order: into a node's slots, through their content, then past the node.
fn position_less(a_path: &[Step], a_index: usize, b_path: &[Step], b_index: usize) -> bool {
    for k in 0..a_path.len().min(b_path.len()) {
        let (a, b) = (&a_path[k], &b_path[k]);
        if a.index != b.index {
            return a.index < b.index;
        }
        if a.slot != b.slot {
            return slot_rank(a.slot) < slot_rank(b.slot);
        }
    }
    if a_path.len() == b_path.len() {
        return a_index < b_index;
    }
    if a_path.len() < b_path.len() {
        return a_index <= b_path[a_path.len()].index;
    }
    a_path[b_path.len()].index < b_index
}

fn move_to_slot(root: &MathList, cursor: &mut MathCursor, next: bool) -> bool {
    clamp(root, cursor);
    let positions = tab_positions(root);
    if positions.is_empty() {
        return false;
    }
    let empty_exists = positions
        .iter()
        .any(|p| !p.exit && list_at(root, &p.path).is_some_and(Vec::is_empty));
    let candidates: Vec<&Position> = positions
        .iter()
        .filter(|p| p.exit || !empty_exists || list_at(root, &p.path).is_some_and(Vec::is_empty))
        .collect();

    // The cursor sits anywhere inside the slot it names; treat it as that
    // slot's stop rather than as a raw position between atoms.
    let path = cursor.path.clone();
    let index = candidates
        .iter()
        .find(|p| !p.exit && p.path == path)
        .map_or(cursor.index, |p| p.index);
    let target = if next {
        candidates
            .iter()
            .position(|p| position_less(&path, index, &p.path, p.index))
            .unwrap_or(0)
    } else {
        candidates
            .iter()
            .rposition(|p| position_less(&p.path, p.index, &path, index))
            .map_or(candidates.len() - 1, |index| index)
    };
    cursor.path = candidates[target].path.clone();
    cursor.index = candidates[target].index;
    true
}

/// Moves to the next Tab stop: an empty slot first when one exists, and
/// always each structural node's exit after its last slot, so a template can
/// be left behind with one more Tab.
pub fn slot_next(root: &MathList, cursor: &mut MathCursor) -> bool {
    move_to_slot(root, cursor, true)
}

/// The mirror of [`slot_next`] for Shift+Tab.
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
    fn a_word_becomes_its_structure_on_space() {
        let mut root = sym("sqrt");
        let mut cursor = at(4);
        assert!(insert_word(&mut root, &mut cursor));
        assert_eq!(root, vec![sqrt(Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 0));

        let mut root = sym("sum");
        let mut cursor = at(3);
        assert!(insert_word(&mut root, &mut cursor));
        assert_eq!(root, vec![big_op(BigOp::Sum, Vec::new(), Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Lower)], 0));
    }

    #[test]
    fn accent_words_build_their_semantic_nodes() {
        for (word, kind) in [
            ("vec", AccentKind::Vector),
            ("dot", AccentKind::Dot),
            ("ddot", AccentKind::DoubleDot),
            ("dddot", AccentKind::TripleDot),
        ] {
            let mut root = sym(word);
            let mut cursor = at(word.chars().count());

            assert!(insert_word(&mut root, &mut cursor));
            assert_eq!(root, vec![accent(kind, Vec::new())]);
            assert_eq!(cursor, at_path(&[(0, Slot::Body)], 0));
        }
    }

    #[test]
    fn contour_integral_word_and_palette_build_a_two_slot_operator() {
        let expected = big_op(BigOp::ContourIntegral, Vec::new(), Vec::new());

        let mut root = sym("oint");
        let mut cursor = at(4);
        assert!(insert_word(&mut root, &mut cursor));
        assert_eq!(root, vec![expected.clone()]);
        assert_eq!(cursor, at_path(&[(0, Slot::Lower)], 0));
        assert_eq!(root[0].slots(), vec![Slot::Lower, Slot::Upper]);

        let mut root = sym("oint");
        let mut cursor = at(4);
        assert!(insert_structure(&mut root, &mut cursor, "oint"));
        assert_eq!(root, vec![expected]);
        assert_eq!(cursor, at_path(&[(0, Slot::Lower)], 0));
    }

    #[test]
    fn a_word_inside_an_identifier_is_left_alone() {
        let mut root = sym("resum");
        let mut cursor = at(5);
        assert!(!insert_word(&mut root, &mut cursor));
        assert_eq!(root, sym("resum"));

        let mut root = sym("1sum");
        let mut cursor = at(4);
        assert!(!insert_word(&mut root, &mut cursor));
        assert_eq!(root, sym("1sum"));
    }

    #[test]
    fn the_word_before_the_cursor_stops_at_a_digit() {
        let root = sym("alpha2beta");

        assert_eq!(word_before(&root, &at(6)), None);
        assert_eq!(word_before(&root, &at(10)), Some("beta".to_owned()));
    }

    #[test]
    fn accepting_a_symbol_replaces_the_whole_word() {
        let mut root = sym("alpha");
        let mut cursor = at(5);

        accept_symbol(&mut root, &mut cursor, 'α');

        assert_eq!(root, sym("α"));
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn taking_the_word_leaves_the_cursor_where_it_began() {
        let mut root = sym("frac");
        let mut cursor = at(4);

        take_word(&mut root, &mut cursor);
        insert_fraction(&mut root, &mut cursor);

        assert_eq!(root, vec![frac(Vec::new(), Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Num)], 0));
    }

    #[test]
    fn completions_offer_symbols_before_structures() {
        let found = completions("s");

        assert!(matches!(
            found.first(),
            Some(Completion::Symbol { name, .. }) if *name != "sqrt"
        ));
        let first_structure = found
            .iter()
            .position(|completion| matches!(completion, Completion::Structure { .. }))
            .expect("structures must be offered");
        assert!(
            found[first_structure..]
                .iter()
                .all(|completion| matches!(completion, Completion::Structure { .. }))
        );
    }

    #[test]
    fn an_exact_structure_outranks_its_prefixes() {
        let found = completions("su");

        let structures: Vec<&str> = found
            .iter()
            .filter_map(|completion| match completion {
                Completion::Structure { name, .. } => Some(*name),
                Completion::Symbol { .. } => None,
            })
            .collect();
        assert_eq!(structures, ["sum", "sup", "sub"]);
    }

    #[test]
    fn every_space_trigger_is_also_a_palette_structure() {
        for (word, _) in WORDS {
            assert!(
                STRUCTURES.iter().any(|structure| structure.name == *word),
                "{word} must be a palette structure"
            );
        }
    }

    #[test]
    fn a_structure_completion_builds_in_place_of_the_word() {
        let mut root = sym("sqrt");
        let mut cursor = at(4);

        assert!(insert_structure(&mut root, &mut cursor, "sqrt"));

        assert_eq!(root, vec![sqrt(Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 0));
    }

    #[test]
    fn an_accent_completion_builds_in_place_of_the_word() {
        let mut root = sym("dddot");
        let mut cursor = at(5);

        assert!(insert_structure(&mut root, &mut cursor, "dddot"));

        assert_eq!(root, vec![accent(AccentKind::TripleDot, Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 0));
    }

    #[test]
    fn frac_captures_the_operand_before_the_word() {
        let mut root = sym("1frac");
        let mut cursor = at(5);

        assert!(insert_structure(&mut root, &mut cursor, "frac"));

        assert_eq!(root, vec![frac(sym("1"), Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Den)], 0));
    }

    #[test]
    fn a_script_completion_builds_a_script_on_an_empty_base() {
        // The word before the cursor is one identifier, so the whole of it
        // goes; the script's base slot is where typing continues, Tab walks
        // to the script slot.
        let mut root = sym("sup");
        let mut cursor = at(3);

        assert!(insert_structure(&mut root, &mut cursor, "sup"));

        assert_eq!(root, vec![script(Vec::new(), Some(Vec::new()), None)]);
        assert_eq!(cursor, at_path(&[(0, Slot::Base)], 0));
    }

    #[test]
    fn a_group_completion_opens_a_bracket_group() {
        let mut root = Vec::new();
        let mut cursor = MathCursor::default();
        insert_char(&mut root, &mut cursor, 'p');
        insert_char(&mut root, &mut cursor, 'a');
        insert_char(&mut root, &mut cursor, 'r');
        insert_char(&mut root, &mut cursor, 'e');
        insert_char(&mut root, &mut cursor, 'n');

        assert!(insert_structure(&mut root, &mut cursor, "paren"));

        assert_eq!(root, vec![group('(', ')', Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 0));
    }

    #[test]
    fn an_unknown_structure_name_is_refused() {
        let mut root = sym("ab");
        let mut cursor = at(2);

        assert!(!insert_structure(&mut root, &mut cursor, "nope"));

        assert_eq!(root, sym("ab"));
        assert_eq!(cursor, at(2));
    }

    #[test]
    fn a_limit_has_no_upper_slot() {
        let node = big_op(BigOp::Limit, Vec::new(), Vec::new());

        assert_eq!(node.slots(), vec![Slot::Lower]);
        assert_eq!(node.slot(Slot::Upper), None);
    }

    #[test]
    fn an_opener_makes_an_empty_group_and_enters_it() {
        let mut root = Vec::new();
        let mut cursor = MathCursor::default();

        assert!(insert_group(&mut root, &mut cursor, '('));

        assert_eq!(root, vec![group('(', ')', Vec::new())]);
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 0));
    }

    #[test]
    fn a_group_wraps_what_comes_next_not_what_came_before() {
        let mut root = sym("x");
        let mut cursor = at(1);

        assert!(insert_group(&mut root, &mut cursor, '('));

        assert_eq!(root, vec![MathNode::Sym('x'), group('(', ')', Vec::new())]);
        assert_eq!(cursor, at_path(&[(1, Slot::Body)], 0));
    }

    #[test]
    fn typing_the_closer_steps_out_rather_than_inserting() {
        let mut root = vec![group('(', ')', sym("x"))];
        let mut cursor = at_path(&[(0, Slot::Body)], 1);

        assert!(close_group(&mut root, &mut cursor, ')'));

        assert_eq!(root, vec![group('(', ')', sym("x"))]);
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn a_closer_that_matches_nothing_is_not_consumed() {
        let mut root = vec![group('[', ']', sym("x"))];
        let mut cursor = MathCursor::default();

        assert!(!close_group(&mut root, &mut cursor, ')'));
        cursor = at_path(&[(0, Slot::Body)], 1);
        assert!(!close_group(&mut root, &mut cursor, ')'));
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 1));
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
    fn the_caret_trigger_wraps_the_preceding_operand() {
        let mut root = sym("x");
        let mut cursor = at(1);
        insert_script(&mut root, &mut cursor, Slot::Sup);

        assert_eq!(root, vec![script(sym("x"), Some(Vec::new()), None)]);
        assert_eq!(cursor, at_path(&[(0, Slot::Sup)], 0));
    }

    #[test]
    fn a_second_trigger_fills_the_other_script_slot() {
        let mut root = sym("x");
        let mut cursor = at(1);
        insert_script(&mut root, &mut cursor, Slot::Sup);
        insert_char(&mut root, &mut cursor, '2');
        assert!(pop_level(&root, &mut cursor));
        insert_script(&mut root, &mut cursor, Slot::Sub);

        assert_eq!(
            root,
            vec![script(sym("x"), Some(sym("2")), Some(Vec::new()))]
        );
        assert_eq!(cursor, at_path(&[(0, Slot::Sub)], 0));
    }

    #[test]
    fn a_second_trigger_typed_straight_through_attaches_to_the_same_base() {
        let mut root = sym("x");
        let mut cursor = at(1);
        insert_script(&mut root, &mut cursor, Slot::Sup);
        insert_char(&mut root, &mut cursor, '2');
        insert_script(&mut root, &mut cursor, Slot::Sub);
        insert_char(&mut root, &mut cursor, 'i');

        assert_eq!(root, vec![script(sym("x"), Some(sym("2")), Some(sym("i")))]);
        assert_eq!(cursor, at_path(&[(0, Slot::Sub)], 1));
    }

    #[test]
    fn a_trigger_nests_when_the_slot_is_already_taken() {
        let mut root = vec![script(sym("x"), Some(sym("2")), Some(sym("i")))];
        let mut cursor = at_path(&[(0, Slot::Sup)], 1);
        insert_script(&mut root, &mut cursor, Slot::Sub);

        assert_eq!(
            root,
            vec![script(
                sym("x"),
                Some(vec![script(sym("2"), None, Some(Vec::new()))]),
                Some(sym("i")),
            )]
        );
        assert_eq!(cursor, at_path(&[(0, Slot::Sup), (0, Slot::Sub)], 0));
    }

    #[test]
    fn an_underscore_no_longer_extends_an_identifier() {
        let mut root = sym("R");
        let mut cursor = at(1);
        insert_script(&mut root, &mut cursor, Slot::Sub);

        assert_eq!(root, vec![script(sym("R"), None, Some(Vec::new()))]);
        assert_eq!(cursor, at_path(&[(0, Slot::Sub)], 0));
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
    fn backspace_reverts_a_script_to_its_literal_atoms() {
        let mut root = vec![script(sym("x"), Some(sym("2")), Some(sym("i")))];
        let mut cursor = at(1);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("x^2_i"));
        assert_eq!(cursor, at(2));
    }

    #[test]
    fn backspace_reverts_a_group_to_its_literal_brackets() {
        let mut root = vec![group('(', ')', sym("x"))];
        let mut cursor = at(1);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);

        assert_eq!(root, sym("(x)"));
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn backspace_reverts_an_accent_to_its_keyword_and_body() {
        let mut root = vec![accent(AccentKind::DoubleDot, sym("x"))];
        let mut cursor = at(1);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);

        assert_eq!(root, sym("ddotx"));
        assert_eq!(cursor, at(4));
    }

    #[test]
    fn backspace_reverts_a_big_operator_to_its_letters() {
        let mut root = vec![big_op(BigOp::Sum, sym("i=0"), sym("n"))];
        let mut cursor = at(1);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("sum_i=0^n"));
        assert_eq!(cursor, at(3));
    }

    #[test]
    fn backspace_reverts_a_contour_integral_to_its_letters() {
        let mut root = vec![big_op(BigOp::ContourIntegral, sym("C"), sym("R"))];
        let mut cursor = at(1);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("oint_C^R"));
        assert_eq!(cursor, at(4));
    }

    #[test]
    fn backspace_in_an_empty_fraction_template_restores_the_slash() {
        let mut root = vec![frac(Vec::new(), Vec::new())];
        let mut cursor = at_path(&[(0, Slot::Num)], 0);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("/"));
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn backspace_in_a_captured_fraction_restores_the_numerator_and_slash() {
        let mut root = vec![frac(sym("12"), Vec::new())];
        let mut cursor = at_path(&[(0, Slot::Den)], 0);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("12/"));
        assert_eq!(cursor, at(3));
    }

    #[test]
    fn backspace_in_an_empty_script_slot_restores_the_trigger() {
        let mut root = vec![script(sym("x"), Some(Vec::new()), None)];
        let mut cursor = at_path(&[(0, Slot::Sup)], 0);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("x^"));
        assert_eq!(cursor, at(2));
    }

    #[test]
    fn backspace_in_an_empty_named_structure_restores_its_word() {
        let mut root = vec![sqrt(Vec::new())];
        let mut cursor = at_path(&[(0, Slot::Body)], 0);

        assert_eq!(backspace(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("sqrt"));
        assert_eq!(cursor, at(4));
    }

    #[test]
    fn backspace_at_the_start_of_a_nonempty_slot_only_climbs() {
        let mut root = vec![frac(sym("a"), sym("b"))];
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
    fn delete_forward_removes_the_current_sym() {
        let mut root = sym("ab");
        let mut cursor = at(0);

        assert_eq!(delete_forward(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("b"));
        assert_eq!(cursor, at(0));
    }

    #[test]
    fn delete_forward_reverts_the_current_structure() {
        let mut root = vec![frac(sym("12"), sym("34"))];
        let mut cursor = at(0);

        assert_eq!(delete_forward(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, sym("12/34"));
        assert_eq!(cursor, at(3));
    }

    #[test]
    fn delete_forward_at_a_nested_slot_end_finds_the_next_atom() {
        let mut root = vec![frac(sym("a"), Vec::new()), MathNode::Sym('z')];
        let mut cursor = at_path(&[(0, Slot::Num)], 1);

        assert_eq!(delete_forward(&mut root, &mut cursor), Removed::Edited);
        assert_eq!(root, vec![frac(sym("a"), Vec::new())]);
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn delete_forward_at_the_root_end_reports_at_end() {
        let mut root = sym("a");
        let mut cursor = at(1);

        assert_eq!(delete_forward(&mut root, &mut cursor), Removed::AtEnd);
        assert_eq!(root, sym("a"));
        assert_eq!(cursor, at(1));
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
    fn arrows_walk_a_script_s_slots_in_document_order() {
        let root = vec![
            MathNode::Sym('a'),
            script(sym("b"), Some(sym("c")), Some(sym("d"))),
            MathNode::Sym('e'),
        ];
        let mut cursor = MathCursor::default();

        for expected in [
            at(1),
            at_path(&[(1, Slot::Base)], 0),
            at_path(&[(1, Slot::Base)], 1),
            at_path(&[(1, Slot::Sup)], 0),
            at_path(&[(1, Slot::Sup)], 1),
            at_path(&[(1, Slot::Sub)], 0),
            at_path(&[(1, Slot::Sub)], 1),
            at(2),
            at(3),
        ] {
            assert!(move_right(&root, &mut cursor));
            assert_eq!(cursor, expected);
        }
        assert!(!move_right(&root, &mut cursor));
    }

    #[test]
    fn arrows_and_tab_reach_a_group_body() {
        let root = vec![group('(', ')', sym("x"))];
        let mut cursor = MathCursor::default();

        assert!(move_right(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 0));
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn arrows_and_tab_walk_an_accent_body() {
        let root = vec![accent(AccentKind::Vector, sym("x"))];
        let mut cursor = MathCursor::default();

        assert!(move_right(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 0));
        assert!(move_right(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 1));
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));
        assert!(move_left(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Body)], 1));
    }

    #[test]
    fn tab_from_a_mid_slot_position_exits_its_template() {
        let root = vec![group('(', ')', sym("x+1"))];
        let mut cursor = at_path(&[(0, Slot::Body)], 0);

        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));
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
        assert_eq!(cursor, at_path(&[(0, Slot::Den)], 1));
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Den), (0, Slot::Num)], 0));
    }

    #[test]
    fn tab_walks_all_slots_when_none_are_empty() {
        let root = vec![frac(vec![frac(sym("a"), sym("b"))], sym("c"))];
        let mut cursor = MathCursor::default();
        let paths = [
            at_path(&[(0, Slot::Num), (0, Slot::Num)], 1),
            at_path(&[(0, Slot::Num), (0, Slot::Den)], 1),
            at_path(&[(0, Slot::Num)], 1),
            at_path(&[(0, Slot::Den)], 1),
            at(1),
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
    fn tab_visits_an_empty_script_slot() {
        let root = vec![script(sym("x"), Some(Vec::new()), None)];
        let mut cursor = MathCursor::default();

        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Sup)], 0));
    }

    #[test]
    fn tab_walks_a_big_operator_s_limits() {
        let root = vec![big_op(BigOp::Sum, Vec::new(), Vec::new())];
        let mut cursor = MathCursor::default();

        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Lower)], 0));
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Upper)], 0));
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn tab_exits_a_filled_template() {
        let mut root = vec![group('(', ')', sym("x"))];
        let mut cursor = at_path(&[(0, Slot::Body)], 1);
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));

        root = vec![big_op(BigOp::Sum, sym("i=0"), sym("n"))];
        cursor = at_path(&[(0, Slot::Upper)], 1);
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));

        root = vec![sqrt(sym("x"))];
        cursor = at_path(&[(0, Slot::Body)], 1);
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));

        root = vec![script(sym("x"), Some(sym("2")), Some(sym("i")))];
        cursor = at_path(&[(0, Slot::Sub)], 1);
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn tab_exits_nested_templates_one_level_at_a_time() {
        let root = vec![frac(sym("1"), vec![group('(', ')', sym("2"))])];
        let mut cursor = at_path(&[(0, Slot::Den), (0, Slot::Body)], 1);

        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Den)], 1));
        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));
    }

    #[test]
    fn shift_tab_retraces_the_exits_in_reverse() {
        let root = vec![frac(sym("a"), sym("b"))];
        let mut cursor = at(1);

        assert!(slot_prev(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Den)], 1));
        assert!(slot_prev(&root, &mut cursor));
        assert_eq!(cursor, at_path(&[(0, Slot::Num)], 1));
    }

    #[test]
    fn tab_from_a_mid_slot_position_moves_on_not_into_the_same_slot() {
        let root = vec![group('(', ')', sym("x+1"))];
        let mut cursor = at_path(&[(0, Slot::Body)], 0);

        assert!(slot_next(&root, &mut cursor));
        assert_eq!(cursor, at(1));
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
