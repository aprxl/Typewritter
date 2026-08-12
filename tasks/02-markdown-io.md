# Task 02 — Markdown parse + serialize

**Wave 2.** Depends on task 01 (the `Document` model exists). You own
**only** `src/document/markdown.rs`. The file currently contains a
doc-comment stub; fill it. Do not touch `mod.rs` — put `impl Document`
blocks in this file (legal: same crate).

**Read first**: `tasks/00-overview.md` (invariants §3 — determinism is a
correctness requirement), `docs/SPEC.md` §9 (serialization), and
`src/document/mod.rs` as merged from task 01.

---

## 1. What the format is

Canonical Markdown, one deterministic spelling per document (SPEC §9:

> Round-trip is property-tested from day one: print∘parse is identity on
> trees; parse∘print is byte-identical on any file we wrote.

). The block/inline grammar below is the *whole* supported surface for this
milestone. Anything the parser does not recognize is literal paragraph
text — there is no error path.

## 2. Block grammar (parse)

Split the file on `'\n'`; strip one trailing `'\r'` per line (CRLF
tolerance). Then:

1. **Heading**: a line matching `^(#{1,3})\s+(.+)$` →
   `Block::Heading { level, content: parse_inline(rest) }`. A heading is
   always a singleton block (one line).
2. **Blank line** (empty or whitespace-only): block separator.
3. **Paragraph**: a maximal run of consecutive non-blank, non-heading
   lines → one `Block::Paragraph`; the lines are joined with a single
   space before inline parsing. (Canonical form: paragraphs re-flow to one
   source line on save. Document this as deliberate.)
4. `#nospace` is a paragraph. `#### four` is a paragraph. Empty input (or
   all blank) → the single-empty-paragraph document from model invariant 1.

## 3. Inline grammar (parse)

A left-to-right scanner producing `Vec<Inline>`:

| Marker | Produces | Rule |
|---|---|---|
| `\` + `*` or `\` | literal `*` / `\` | any other char after `\` keeps the backslash literal |
| `***x***` | bold+italic | |
| `**x**` | bold | |
| `*x*` | italic | |
| `_x_` | italic | parses, but serializes as `*x*` (canonical) |

Rules that keep it sane:

- An opening marker must be **immediately followed by a non-whitespace
  char**; otherwise the marker chars are literal (`* nope*` is plain).
- The matching closer is the next occurrence of the same marker; the char
  immediately before the closer must be non-whitespace. No closer → the
  opening marker is literal text.
- **No nesting** beyond `***`: `**a *b* c**` parses as bold `a *b* c`
  (the inner markers are literal content of the bold run). Flat runs only.
- Runs produced by parsing are exactly the styles found; merge adjacent
  same-style runs if your scanner naturally emits them, but the model
  tolerates both.

## 4. Serialization

```rust
impl Document {
    pub fn load(path: &Path) -> Option<Document>;   // see §5
    pub fn save(&mut self) -> io::Result<()>;       // clears dirty on success
}

pub fn parse(path: &Path, text: &str) -> Document;  // pure, testable
pub fn serialize(doc: &Document) -> String;         // pure, testable
```

`serialize`:

- Blocks joined by `"\n\n"`; file ends with exactly one trailing `"\n"`.
- Heading: `"#" * level + " " + serialize_inline(content)`.
- Paragraph: `serialize_inline(runs)`.
- Inline: escape first (`\` → `\\`, `*` → `\*`; additionally, if a
  paragraph/heading's serialized line would start with `#`, escape it
  `\#`), then wrap each run: bold+italic → `***…***`, bold → `**…**`,
  italic → `*…*`, plain → as-is. Empty runs emit nothing.
- Empty document (one empty paragraph) serializes to `""`? No: blocks
  joined + trailing newline of a single empty line is `"\n"` — pick one
  and test it. Recommended: `""` (empty file) parses back to one empty
  paragraph, so `""` is the fixpoint. Use `""`.

## 5. Loading

`Document::load(path)`:

- `fs::read` → `None` if unreadable.
- `None` if the bytes contain a NUL (`0x00`) — the binary marker; images
  and PDFs must not open (this behavior already exists in
  `Buffer::load` — keep it).
- Otherwise `String::from_utf8_lossy` → `parse`.

## 6. Tests

Property tests (hand-rolled, no new dependencies):

1. **AST round-trip**: for a fixture list of hand-built `Document`s —
   empty; plain paragraph; heading 1/2/3; bold; italic; bold+italic;
   text containing literal `*`, `\`, leading `#`, adjacent same-style
   runs, `* spaced out *` markers — assert `parse(serialize(d)) == d`
   (derive `PartialEq` on the model; `path`/`name`/`caret` excluded —
   compare `blocks`).
2. **Canonical stability**: for a fixture list of Markdown strings,
   assert `serialize(parse(serialize(parse(s)))) == serialize(parse(s))`
   — i.e. print(parse(print(ast))) == print(ast).
3. **Table tests** for the parse edge cases: `#5`, `#`, `#### x`,
   `*no close`, `**no close`, `* space*`, `\*literal`, `**a *b* c**`,
   CRLF input, blank-line-only file, multi-line paragraph joining.
4. `save`/`load` round-trip through a real temp file, and `load` rejects
   a NUL-containing file.

## 7. Verification

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

## 8. Non-goals

- Full CommonMark. No lists, links, code, quotes, setext headings,
  `__bold__`, strikethrough.
- The fenced `tw-math` block format (§9 of the spec) — its own milestone.
- Wiring `load`/`save` into the app (task 05).
