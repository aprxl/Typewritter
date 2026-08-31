# PDF export

The plan for §10 of [`docs/SPEC.md`](docs/SPEC.md): a PDF that is the
editor's own page, not a transcript of it.

Read [§1](#1-the-shape-of-it) before touching anything. Read
[§6](#6-adding-an-element) before adding an element — that is the whole
recipe, and it is short on purpose.

---

## 1. The shape of it

Direct. `Document` → PDF, no HTML in the middle. That is not a preference
about markup; it is forced by the spec's hardest line:

> **One layout engine, not two.** Screen and PDF render through the same
> path or the promise quietly breaks. — `docs/SPEC.md:276`

An HTML detour would mean a second layout engine (the browser's) deciding
where lines break, and no amount of CSS makes it agree with
`document/layout.rs` about a wrapped heading or a math atom's width. The
direct pipeline has exactly one engine because it *reuses* the one that
already exists.

```
Document                              the open note, blocks and inlines
   │
   │  document::layout::layout(doc, MEASURE, measure)      ← UNCHANGED, SHARED
   ▼
DocLayout                             blocks → VisLines → Segments, logical px
   │
   │  export::paginate::paginate(&layout, &geometry)
   ▼
Vec<Page>                             each a list of Pieces: (block, line range, y)
   │
   │  export::paint::page(canvas, ..)  one match arm per Block kind
   ▼
Canvas calls                          draw_rectangle / draw_circle / draw_path / draw_text
   │
   │  export::pdf::PdfCanvas           krilla
   ▼
PDF bytes
```

The load-bearing observation is that **`document/layout.rs` is already
pure**. `layout(doc, width, measure) -> DocLayout` measures nothing itself
— it takes a closure — and knows nothing about a `Layer`, a scroll offset,
or a caret. The editor is *a painter of `DocLayout`*, not its owner. So
export does not need the layout engine changed, generalised, or abstracted.
It needs a second painter.

### What is shared, and what is not

| | Screen | PDF |
|---|---|---|
| Block model, wrapping, math boxes, list indents, fold arithmetic | `document::layout` | **same code** |
| Text measurement | `theme::width` → `Layer` | **same code** |
| Glyphs, faces, faux bold/italic/condense | app text stack | **same shaper** (§4) |
| Colours, sizes, fonts | `theme` | **same tables** |
| Painting | `components/editor.rs` | `export/paint.rs` |
| Caret, selection, current-line band, hover, scroll, fold chevrons | drawn | **not drawn** |
| Pagination | none (one scroll) | `export/paginate.rs` |

The right column is deliberately short. Everything that decides *where
something is* is shared; only *what gets ink* differs, and it differs by
subtraction — editor chrome is state about a session, not content of a
document, and a PDF has no session.

### Why not a `Canvas` trait that `Layer` also implements

Considered and deferred. It would let `editor.rs` and `paint.rs` be one
function, which sounds like the strongest possible 1:1 guarantee. It is
not worth it yet: `Editor::draw` is 600 lines of caret, selection,
brush, math cursor and scroll culling, none of which a page has, and
threading all of it through a trait to then branch it off again buys
nothing. The `Canvas` trait exists anyway (§3) and its methods are named
and shaped **exactly** like `Layer`'s, so `impl Canvas for Layer` stays a
one-file change if we ever want it, and `paint.rs` reads as a diff against
`editor.rs` in the meantime.

---

## 2. The page

### Decisions (April, 2026-08-30)

1. **Book size.** `PT_PER_PX = 0.63`. The editor's column is a fixed
   `editor::MEASURE = 634` logical px, so the PDF lays out at exactly that
   width and the line breaks are the editor's, character for character.
   `0.63` maps that column to 399.4 pt and body text to 11.0 pt.
2. **Asymmetric page only when the note has sidenotes.** A note without
   them gets a centred column. A note with them gets column + margin
   column and a smaller scale, so both fit the paper.
3. **Folds always expand.** A collapsed section prints in full; folding is
   a reading aid, not a property of the document. This closes
   `docs/SPEC.md` §13 open question 1. No `fold_indicator` is ever painted,
   and `paginate` walks `source` order, not visible order.

### Geometry

A4 portrait, 595.276 × 841.890 pt. Two constructors, one struct:

```
PageGeometry::plain()                 no sidenotes
    column     634 px  ×  0.63  =  399.42 pt
    side       (595.276 − 399.42) / 2  =  97.93 pt
    top/bottom 72 pt each
    content    697.89 pt  =  1107.8 px  ≈  37 body lines

PageGeometry::noted()                 sidenotes present — NOT YET BUILT
    spread     MEASURE + RIGHT_MARGIN + sidenotes::WIDTH
               634 + 24 + 215  =  873 px
    side       40 pt
    pt_per_px  (595.276 − 80) / 873  =  0.590   → body 10.3 pt
```

`noted()` prints smaller, which is the honest trade: a page carrying
sidenotes carries more. If it reads too small in practice the escape hatch
is the paper, not the scale — landscape A4 or A3 — and that is a
`Paper` field, not a redesign.

**The page is the only place `pt` exists.** Everything upstream —
pagination, painting, every constant in `paint.rs` — is in the editor's
logical pixels, with the editor's top-left origin and y pointing down.
`PdfCanvas` multiplies by `pt_per_px` and adds the margin at the moment it
touches krilla, and nowhere else. krilla's surface is also top-left
y-down, so there is no flip anywhere in the pipeline; if you find yourself
writing one, something is wrong.

### Theme

Export forces `Theme::LIGHT` for the duration and restores the caller's
theme afterwards. `theme::set` is a global (`RwLock<ThemeServer>`) and
`layout::text_style` reads ink colours while laying out, so this has to
wrap the layout pass as well as the paint pass. Export is synchronous on
the main thread and nothing else draws during it, so the swap is not
observable.

Strict 1:1 would print a dark note on black. It is a knob
(`Options::theme`) rather than a law, defaulting to light, because
printing a dark page is a choice almost nobody makes on purpose and the
one who does can say so.

---

## 3. Modules

Everything lives in `src/export/`. One concern per file, per AGENTS.md.

| File | Owns |
|---|---|
| `mod.rs` | `Options`, `export_pdf` — the whole public surface, one call |
| `geometry.rs` | `Paper`, `PageGeometry`, px → pt. The only file that says `pt` |
| `canvas.rs` | The `Canvas` trait. Backend-agnostic, `Layer`-shaped |
| `text.rs` | `Shaper`: measurement and shaped glyph runs from the app's text stack |
| `paginate.rs` | `Piece`, `Page`, `paginate`. Where a page breaks and what lands on it |
| `paint.rs` | `DocLayout` → `Canvas` calls. **The extension point** |
| `pdf.rs` | `PdfCanvas` — the krilla backend, and the font cache that feeds it |

Plus, in the vendored platform:

| File | Addition |
|---|---|
| `renderer/text_stack.rs` | `shape` — the glyphs behind a measurement |
| `renderer/layer.rs` | `Layer::shape_text`, `Layer::face_data` |

### `Canvas`

```rust
pub trait Canvas {
    fn draw_rectangle(&mut self, at: (f32, f32), size: (f32, f32), color: Color, r: Rounding);
    fn draw_circle(&mut self, center: (f32, f32), radius: f32, color: Color);
    fn draw_path(&mut self, d: &str, at: (f32, f32), scale: f32, paint: &PathPaint);
    fn draw_text(&mut self, text: &str, at: (f32, f32), style: &TextStyle, align: Alignment);
}
```

Four methods, all in logical pixels, all named and shaped like `Layer`'s.
That is the entire abstraction. A new backend is a new `impl`; a new
element never touches this file.

---

## 4. Text: why the glyphs are the same glyphs

This is the part that earns the "1:1", and the part most likely to be got
wrong by someone who skips it.

The app does **not** draw text the way a normal cosmic-text consumer does.
Georgia ships one cut here, so bold, italic and condensed are all
synthesised at rasterisation by `renderer/glyph_effects.rs`: `embolden`
dilates the outline, `transform` shears and x-scales it, and the pen is
corrected per glyph by `effect_advance_delta`. `TextStack::measure` applies
the *same* correction, which is why a measured width matches a drawn one.

A PDF that shaped its own text — with rustybuzz, with krilla's
`simple-text`, with anything — would be a second shaper making a second
set of decisions, and the promise would be broken at the first bold
heading. So export does not shape. It asks the app's own text stack for the
glyphs it already shaped, and hands them to krilla positioned:

```
renderer::ShapedText {
    glyphs: Vec<ShapedGlyph { face, glyph, x, advance, cluster }>,  // logical px
    baseline: f32,        // from the run's box top
    size: (f32, f32),     // == get_text_size, exactly
}
```

`Layer::shape_text` scales by the DPI factor, shapes, and divides back out
— the identical dance `get_text_size` does — so a glyph's `x` and the
width the layout engine wrapped on come from one number.

The three synthetic effects then map onto PDF exactly, with no
approximation:

| Effect | Screen | PDF |
|---|---|---|
| faux bold (`weight`) | outline dilated by `w` px each side, pen `+2w` | fill **and** stroke the glyph run, stroke width `2w` |
| faux italic (`slant`) | outline sheared about the glyph origin | shear about the baseline — every origin is on it, so identical |
| faux condense (`width`) | outline x-scaled about the glyph origin, advance × `c` | x-scale the run about its start — pen is `Σ c·adv = c·Σ adv`, so identical |
| tracking | half a track shifted into each glyph | folded into the glyph x positions |

Bold is per-run state on the surface; slant and condense are one
transform pushed around the run. Both are constant within a segment, which
is what makes "about the run" and "about each glyph" the same answer.

The pen arithmetic is not re-derived — `paint`/`pdf.rs` call
`glyph_effects::effect_advance_delta`, the function the screen measures
and draws with. There must never be a second copy of that formula.

### Font embedding

The only thing that knows where Georgia lives is the `FontSystem`'s
`fontdb`, which already resolved and loaded it. `Layer::face_data(face)`
returns those bytes plus the face index; `pdf.rs` caches one krilla `Font`
per face for the export. This also gets font *fallback* right for free: a
glyph the body face lacks comes back tagged with whichever face
cosmic-text actually fell back to, so the PDF embeds what the screen drew
rather than a guess.

krilla subsets what it embeds and writes `ToUnicode` from the cluster
ranges, so the output stays small, selectable and searchable.

---

## 5. Pagination

`paginate(&DocLayout, &PageGeometry) -> Vec<Page>`, where

```rust
struct Piece { block: usize, lines: Range<usize>, y: f32 }   // y: on-page, px
struct Page  { pieces: Vec<Piece> }
```

A piece is *a block, a slice of its lines, and where the slice lands*. One
type covers both a whole block and a paragraph split across a break, so
the painter has one path, not two.

Rules, in order:

1. Break between blocks. A block that fits on the rest of the page goes
   there; otherwise the page ends.
2. A block taller than a whole page splits between its `VisLine`s.
   Paragraphs, list items and code may split; nothing else reaches this
   rule.
3. **Keep-together**: `Block::Math`, and a run of `Block::CodeLine` with
   one `first: true` at its head, are atomic. They move to the next page
   whole rather than split.
4. **Keep-with-next**: a `Block::Heading` never ends a page. If the block
   after it does not fit, the heading goes with it.
5. Inter-block gaps (`GAP_PARAGRAPH` and friends) are swallowed at a page
   break — a page never opens with leading whitespace.

Only rules 1 and 2 exist today; 3–5 arrive with the elements they protect,
and each is a few lines in one function.

Folded blocks are expanded before pagination (decision 3), so
`BlockLayout::hidden` is ignored and `indicator` is never painted. **This
means export cannot use the editor's `DocLayout`** — the shell's cached
one has folds applied. Export lays out its own from a fold-cleared copy of
the document.

---

## 6. Adding an element

The recipe, and the reason the structure is shaped the way it is.

1. **Nothing in `canvas.rs`, `geometry.rs`, `pdf.rs`, or `text.rs`.** If
   an element makes you want to add a `Canvas` method, check first whether
   the editor draws it with the four that exist. It does — those four are
   everything `editor.rs` uses (`draw_path_rotated` is `draw_path` with an
   angle; add the parameter, not a method).
2. **One arm in `paint::block`.** Its decorations: the tint behind code,
   the bullet beside a list item, the band behind math, the rule of a
   divider. Read the corresponding stretch of `editor.rs` and drop the
   parts that ask about carets or scroll. Keep the constants where they
   are — import them from `editor.rs`, never retype the number.
3. **Inline runs, if it has any, go in `paint::line`** — the shared loop
   every block's text passes through. A new `Inline` variant is an arm
   there.
4. **A keep-together rule in `paginate`, if it must not split.**
5. **A test.** `paint` against a recording `Canvas` — assert the calls,
   not the bytes. `pdf.rs` is exercised once, by the smoke test; asserting
   on PDF output is asserting on krilla.

The reason the seam is `paint::block` and not something cleverer: a block
kind is *already* the unit the document model, the layout engine, and the
editor's painter all agree on. Adding a fifth registry keyed by the same
thing would be indirection with no reader.

### Status

| Element | Layout | Paint | Paginate |
|---|---|---|---|
| Paragraph | ✅ shared | ✅ | ✅ |
| Heading (+ auto-number) | ✅ shared | ✅ | keep-with-next ⬜ |
| bold / italic / plain runs | ✅ shared | ✅ | — |
| Divider | ✅ shared | ✅ | ✅ |
| Inline code, code blocks | ✅ shared | ⬜ | keep-together ⬜ |
| Lists: bullet, number, task | ✅ shared | ⬜ | ✅ |
| Math, inline and display | ✅ shared | ⬜ | keep-together ⬜ |
| Equation numbers | ✅ shared | ⬜ | — |
| Badges | ✅ shared | ⬜ | — |
| Highlights | ✅ shared | ⬜ | — |
| Sidenotes | ✅ shared | ⬜ | repaginate ⬜ |
| Images | — | ⬜ | ⬜ |

Every ⬜ in the Paint column prints its **text** correctly today and only
lacks its decoration: `layout::text_style` already styles every block kind,
and `paint::line` already draws it. A list item prints without its bullet,
not as a hole in the page. That is the layering AGENTS.md asks for — the
product works at every step, and each row above is one commit.

---

## 7. Traps

- **`DocLayout` is in logical pixels with y down.** So is `Canvas`. Only
  `PdfCanvas` knows about points. Do not scale twice.
- **`baseline` in `editor.rs` is the line's vertical *centre*, not a
  typographic baseline** — text is drawn with `VerticalAlign::Center`.
  `paint.rs` uses the same name for the same thing. The real baseline is
  recovered inside `PdfCanvas::draw_text` from the shaped run.
- **`text_style` reads the theme while laying out.** Set the theme before
  `layout`, not between layout and paint.
- **Do not reimplement `effect_advance_delta`.** §4.
- **`equation_numbers` and `anchors` are derived by `layout`, and heading
  numbers by `outline::outline`.** Export derives all three the same way
  the editor does. Never store them.
- **`cargo check` proves nothing about output.** Nor does the suite: the
  painter's tests run against a recording canvas with no font behind it, so
  they check *what is drawn where* and can say nothing about what came out.
  A change to `pdf.rs` or `text.rs` is unverified until a PDF has been made
  and read. The recipe that produced the first one:
  - add a temporary hook in `Shell::update` that loads a note and calls
    `export::render`, run the app per AGENTS.md's smoke test, remove it;
  - inflate the page content streams and read the operators. `Tf` carries
    the font size (body must be `17.5 × scale`), `Tr` the render mode (`2`
    on a bold run, `0` elsewhere), `w` the faux-bold stroke, `Tm` the
    baseline, and `ActualText` the run's text in document order. That is a
    sharper check than looking at a render, and it catches the failures
    that matter — a doubled scale, a lost run, a drifting baseline.

---

## 8. Dependency

`krilla = { version = "0.8", default-features = false }`.

A canvas that speaks pre-shaped glyphs, which is the one thing this
pipeline needs and the thing that makes §4 possible. It handles font
subsetting, CID embedding and `ToUnicode`; writing those by hand against
`pdf-writer` is a few hundred lines of spec-following for no gain.

Default features are off deliberately: `simple-text` would pull rustybuzz
in to shape text we shape ourselves (§4), and `raster-images` is for the
images row of §6, to be turned on when that row is.
