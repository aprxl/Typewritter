# Work log

State at handoff: `cargo fmt --check` clean, `cargo clippy --all-targets -D
warnings` clean, 512 tests passing, and the tree committed as `fc46251`
(`Scale math layout with document`).

How the work is being done, since `AGENTS.md` is out of date on this: coding is
delegated to the `opencode` CLI rather than to in-process subagents. The last
last two sessions used `openrouter/deepseek/deepseek-v4-pro-0813 --variant
high` and then `openrouter/openai/gpt-5.6-luna --variant high`; both carried
design-level work well, and luna is cheaper. Luna needs `openai` allowed under
OpenRouter's allowed-providers setting, or every run fails before it starts. Each task spec names the
exact files that run owns, and no two runs in a wave own the same file; waves
run one at a time, because parallel `cargo` builds contend. A spec must say *do
the work yourself, do not delegate it further* — runs that delegate have
stalled and produced nothing. Runs now make their own commits, and whoever is
driving still verifies `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings` and `cargo test` afterwards rather than trusting the report. When
judging whether a run has stalled, look at the **child** `opencode` process's
CPU time, not the log size — the parent shell buffers, and an empty log on its
own has already caused one wrong diagnosis.

**Read the diff, not the report.** Three of the eight waves below landed green,
passed every check, and still shipped a defect that only reading the code
found: an autosave that retried every frame, a paste that mangled prices, and
an anchor that reserved less width than it drew. Each was caught by reading the
diff and fixed in a follow-up wave. A green `cargo test` says the tests agree
with the code, not that the code is right.

---

## Session log — sidenote editing

A note's text can now be edited with the same editor the page uses. Six waves.

The shape of it: rather than build a second, smaller editor with its own state
and its own key handling, the **editing scope became a property of the
document**. The caret is either in the body or inside one note's body, and
every primitive already written operates on whichever it is. `src/vim/` and
`src/tabs.rs` never learned that notes exist — they call the same `Document`
methods they always called. Undo needed nothing: a `Tab` snapshots the whole
`Document`, and a note body is part of it.

| Commit | Wave |
|---|---|
| `e26d8dc` | `Focus::Body`/`Focus::Note(i)`, note bodies become `Vec<Block>` |
| `9ac62a9` | ↳ `scope()` vs `body()` — the repair, see below |
| `cccff24` | `layout_blocks`, `Editor::Metrics`, margin draws real editors |
| `44b362e` | `note.edit`, click an anchor, click a note, Escape back out |
| `ab5fb94` | ↳ a body coordinate always returns focus to the body |
| `fc46251` | ↳ math sized at the margin's scale |

Three of the six waves needed a follow-up, all found by reading the diff:

- **`e26d8dc` swept half the file.** `flat_to_pos` followed focus while
  `position`, `set_flat_position`, `range_text`, `line_range`, `delete_range`,
  `text_object_range` and `block_text` still indexed the body — and
  `vim/motion.rs` read the public `doc.blocks` field directly in twenty-one
  places. With the caret in a note, `x` and `dw` would have deleted prose. The
  spec caused it by forbidding edits to `src/vim/`. `9ac62a9` made the field
  private so the compiler forced a decision at all ~245 sites: `scope()` is the
  blocks the caret is in, `body()` is the file's own.
- **`44b362e` made Escape the only way out.** Clicking prose while a note was
  focused hit the *page* layout and wrote the result through `move_caret_to`,
  which resolves against the focused scope. `ab5fb94` routes every
  body-derived coordinate through one helper that sets focus and places the
  caret together. Motions stay scope-local: inside a note, `G` ends at the end
  of the note.
- **`cccff24` scaled every text size except math.** `math_layout` sizes off its
  own `BASE_SIZE`, so an expression in a note rendered at full page size and
  overflowed the column — in an app for math-heavy lecture notes. `fc46251`
  threads the scale through, keeping it separate from the `level` that shrinks
  a fraction's operands.

That is the same pattern as the readiness pass: **a body coordinate handed to a
scope-resolving function**, three times, each time passing every check first.

Deliberate limits, both marked in the code: **one paragraph per note** (matches
the one-line footnote definition written to disk; multi-paragraph footnotes are
indented continuation lines, which is the upgrade path) and **no note inside a
note**.

Known and left: a Normal-mode click on prose opens the context menu without
moving the caret, so focus stays in the note. The `*_at` edits it makes are
explicitly body-addressed, so nothing lands in the wrong scope.

**Not visually verified.** Every wave passed the 6-second smoke run with no
panic and no wgpu error, but nothing here has been seen on screen: there is no
headless X server on this machine, and the only display is the user's own
session. The margin at its new scale, the caret blinking in a note, and the
focused note's accent tick are all unwatched.

---

## Session log — readiness pass

The four highest items from the readiness report, done as eight waves.

**`src/shell/input.rs`, `src/components/editor.rs`** — the tree was red. The
brush is additive now, so `toggle_brush_hits` became `add_brush_hits` and three
tests were re-aimed at the invariant that actually survives: a stroke only ever
grows the selection and never holds a duplicate. `BRUSH_RING` and the test that
validated its path data went with the ring itself.

**`src/main.rs`, `src/tabs.rs`, `src/shell/mod.rs`** — nothing was ever written
to disk unless the reader asked. `CloseRequested` ran `event_loop.exit()` and
that was all. `Tabs::save_all` writes every dirty tab with a path, skips the
pathless ones, and returns the failures so one bad write cannot mask the rest.
The shell debounces off the revision counter `Tabs` already bumps, and folds
the pending deadline into `wake_at` beside the animations — without that the
save fires while typing and never once typing stops, which is exactly backwards.
Close saves unconditionally: a dialog between a student and their closing
laptop loses work rather than protecting it.

The follow-up wave matters more than it looks. The clock restarted only when
the revision moved, not when a save was *attempted*, so any tab still dirty
afterwards left the deadline permanently in the past — `about_to_wait` turned
that into an immediate redraw, and the loop spun at full rate with a file write
and an `eprintln!` every frame. Two ways in, both real: a failing write, and a
dirty tab with no path that `save_all` correctly skips. The field is now
`autosave_last_attempt`, because a name describing half of what a value means
is what hid the bug.

**`src/document/mod.rs`, `src/shell/input.rs`** — copying an expression put
U+FFFC on the clipboard, so a copy and paste destroyed the maths. `range_text`
writes the `$…$` form instead. Entering Insert mode also opened an undo
transaction on the `i`/`a`/`o` path but not on `Enter(Mode::Insert)`, which is
how a click into an expression gets there — so the same typing undid one
keystroke at a time or all at once depending on how you arrived. Both routes go
through one `start_insert` now.

The follow-up: reading `$…$` back out of *any* inserted text meant a pasted
`costs $40 and $12` became an expression, and so did `$PATH` and `$HOME` in a
pasted shell line. Notation is now read back only from text this app produced —
`Shell::exported_yank` already held the exact published string and `import_yank`
already compared against it, so the provenance test was written and simply not
being used for this. `insert_text` is literal and takes every keystroke;
`insert_notation` interprets and only ever sees our own bytes. Copy escapes a
literal `$` as `\$`, so a copied range is byte-identical to what the file holds.

**`src/document/mod.rs`, `src/document/markdown.rs`** — sidenotes stopped being
`mock_notes()`. The format is Markdown's own footnote syntax, `[^1]` inline and
`[^1]: body` on its own line, chosen over anything invented because every other
tool already reads it and it stays greppable. Definitions are lifted out of the
block stream into `Document::notes` so they cannot be edited as stray
paragraphs, and written back at the end in anchor order. Labels round-trip
verbatim — renumbering on save would give auto-commit a phantom diff on every
open. An anchor with no definition and a definition with no anchor both survive,
because files get edited by hand and by other tools. The anchor is opaque
exactly like a math atom, so every motion, selection and offset kept working
untouched; adding the variant needed one-line arms in four more files than
expected (`outline.rs`, `shell/input.rs`, `shell/mod.rs`, `vim/motion.rs`).

**`src/components/sidenotes.rs`, `src/document/layout.rs`,
`src/components/editor.rs`, `src/shell/`, `src/tabs.rs`** — and into the
margin. Anchors draw as position-derived raised numbers, never stored, the same
decision the heading outline made and for the same reason. `DocLayout` reports
each anchor's y; `stack` is a pure one-pass resolver that pushes a crowded note
down and never up, so a moved note still sits beside the sentence that anchored
it. It runs in `rebuild_views`, off the typing path per §12, and jumps rather
than animating — a note sliding into place while you type is worse than one
that jumps. `format.sidenote` is in the `/` menu.

The follow-up here was the one the wave itself flagged: `advance` measured an
anchor as one digit while drawing the real number, so from the tenth note on it
overlapped the next character — the same advance-versus-draw drift the badge
work exists to prevent. The ordinal is now stamped on the piece and segment the
layout already builds, and `advance` measures the number actually drawn. The
editor's separate anchors map went away; the segment is the single source.

**Sidenotes are read-only in the margin.** Creating a note and seeing it beside
its anchor is what shipped. Editing a note's text in place is the next piece and
was deliberately not started, because half an editor is worse than none.

---

## Session log — current work

**`src/document/mod.rs`, `src/document/layout.rs`, `src/document/markdown.rs`**
— line dividers. `Block::Divider` holds one always-empty run, satisfying the
"every block has a run" invariant. Typing on a rule turns it back into a
paragraph, enforced centrally in `prune_runs` rather than at each edit site.
Divider metrics are deliberately tighter than a blank line: a big hole
disrupts reading more than the separation is worth. Markdown writes `---`,
with an escape guard for a paragraph that would re-read as a rule.

**`arboard`, `src/shell/`** — OS clipboard. `arboard` was chosen over pulling
in a second UI toolkit. All clipboard I/O lives in the shell, so document and
tab models stay pure and their tests stay headless. The yank register exports
once per frame by comparing state, covering every yank path without a hook at
each one.

**`src/shell/`, `src/components/`** — right-click menus. Research first:
winit has no menu API, and `muda` (the standard cross-platform menu crate)
needs GTK3 and libxdo on Linux, which would link a second UI toolkit into an
app that draws its own title bar. The menu is drawn in-app on the same
detached-overlay pattern as the palette and slash menu; the clipboard goes
through the OS, which is the part that genuinely has to interoperate. Menus
now cover the editor, a file-tree row, and empty tree space.

**`src/document/outline.rs`, `src/components/editor.rs`, `src/components/topics.rs`**
— document outline. One pure function has three consumers: editor margin
numbers, Topics panel, and breadcrumb trail. All derive from the same blocks,
so a number can never disagree with the heading beside it. Headings nest by
insertion order rather than level arithmetic, so a document that opens at H3
or skips a level still comes out sensible. Auto-numbers are virtual: hung in
the margin and drawn rather than laid out, so the caret can never reach one
and it never shifts the heading it labels.

**`MATH.md`, `src/document/math.rs`, `src/document/math_layout.rs`,
`src/document/math_notation.rs`, `src/document/layout.rs`, `src/shell/`** —
math mode. The design lives in `MATH.md`; this log does not restate it. The
math tree uses single-character atoms in the TeX hlist model, so the cursor is
an index and never a string offset. It has cursor and edit operations, `/`
trigger operand capture, and structural revert on backspace. Recursive box
layout makes nesting work because a nested fraction is just a tall child its
ancestors grow to hold. Canonical linear notation is deterministic and
property-tested both ways. In containers, an inline atom costs exactly one
character in flat-text space, so existing motion and selection work unchanged.
Real drawing and content-driven line heights are in place. Shell wiring makes
`/math`, the fraction trigger, Tab between slots, and Esc popping levels work
in the running app. JuliaMono is the math font, chosen for glyph coverage.
Verified end to end in the app: typing `1 / 2 / 3` builds a nested fraction,
saves as `gain is $(19)/(2/3)$`, and reopens as the same tree.

**`src/document/math.rs`, `src/document/math_layout.rs`,
`src/components/editor.rs`** — superscripts and subscripts. `^` and `_` capture
the preceding operand and wrap it in a `Script`, whose `sup` and `sub` are
options rather than always-present lists: a script that has not been asked for
does not exist, so Tab does not walk through empty slots nobody wanted. Because
of that, `MathNode::slots()` returns an owned `Vec` — a script's slot list
depends on which of its scripts exist, so it cannot be a `&'static` slice.
Typing `i^2_2` straight through attaches the second script to the same base
rather than nesting it inside the first, which is what MathQuill and LaTeX both
do and what the fingers expect. The cost is that a script directly inside
another script can no longer be typed in one flow; it needs grouping brackets,
which now exist.

**`src/document/math_layout.rs`** — two placement bugs, one the cause of the
other, both worth recording because the coordinate model is the trap. The
renderer draws glyphs vertically **centred** (`theme::LEFT`), so the anchor line
is a centre line, not a baseline. Placing children by baseline made a fraction's
numerator sit closer to the bar than its denominator. The first fix made every
box symmetric about the anchor line (ascent == descent == height/2) and put the
bar on the anchor line, deleting `AXIS_RISE`, `ASCENT_RATIO` and `DESCENT_RATIO`
— correct for glyphs, wrong for anything lopsided: `C_x` has ascent 8.75 and
descent 15.4, and half-height placement pushed it through the bar. The rule that
actually holds, and that `fraction`, `script`, the delimiters and the big
operators all now follow, is **place a box by the edge that faces whatever it
must clear**. Anything added here has to obey it.

**`src/document/math.rs`, `src/document/math_notation.rs`** — bracket groups.
TeX's split is adopted wholesale: `{ }` is the notation's invisible grouping and
`( )` / `[ ]` are visible `Group` nodes, so the printer's grouping can never be
confused with a bracket the reader typed. This changed the on-disk grouping
character from `( )` to `{ }`; no migration was written, since nothing outside
this machine has files in the old form. Typing a closer steps out of the group
rather than inserting anything, and types literally when there is no group to
leave.

**`src/document/math.rs`, `src/document/math_notation.rs`,
`src/document/math_layout.rs`** — word triggers. `sqrt`, `sum`, `prod`, `int`
and `lim` followed by a space become their structure. A big operator's limits
are always-present lists, unlike a script's options, because a `∑` always has
somewhere to put its bounds — they start empty and are typed into, and that is
what Tab walks. `BigOp::keyword()` is the single place the typed word and the
on-disk word come from, so they cannot drift. The trigger fires on standalone
tokens only, so a variable named `sum` and the `sum` inside `resum` both
survive. On disk a keyword is a structure only when immediately followed by `{`,
which is unambiguous in both directions and still greppable. Layout draws a
delimiter and a radical sign as glyphs scaled from the body height (`DELIM_FILL`,
floored at the text size), an overbar as the existing `Bar`, and a big operator
as a vertical column centred on its widest member — no new `BoxKind` was needed.

**`src/theme.rs`, `src/components/editor.rs`** — a display math block gets its
own slab, drawn in the same pass as a code block's. `theme::MATH` is a sibling
of `CODE` rather than the same tint: cooler and greyer, so the two block kinds
are told apart at a glance without either shouting.

**`src/document/math_symbols.rs`, `src/document/math.rs`** — the symbol table:
79 completions under Greek, Relations, Operators, Arrows, Sets and Calculus,
named the way LaTeX names them, which is the vocabulary this reader arrives
with. Matching is prefix rather than fuzzy with an exact name first, so `in`
offers `∈` ahead of `infty`, and case-sensitive, since case is the only thing
separating `delta` from `Delta`. `word_before` reads the query out of the tree
rather than tracking what was typed, so backspace, an arrow move and a click
elsewhere all just change the answer and there is no state to fall out of step.
`take_word` exists so accepting a structure completion can remove the letters
before firing the trigger — otherwise a fraction captures them as its numerator.

---

## What is left

In priority order, as of `fc46251`. Everything above this line is done.

1. **Full-text search.** `file_finder.rs` is fuzzy *filename* matching only.
   SPEC §7.3 calls search the whole retrieval story and asks for three things,
   none of which exist: math searchable, results readable without opening the
   note with the maths **rendered as maths**, and fuzzy tolerance because the
   notes are rough. The data side is already right — `tw-math v1` fences and
   `$…$` inline are both greppable — so what is missing is the panel. Biggest
   remaining feature by a distance, and the one that makes keeping notes in the
   app worth doing.

2. **Frontmatter `created`, and the tree sort that needs it.** No frontmatter
   parsing exists. `vault.rs` sorts by name, which SPEC §7.2 rules out in as
   many words: a folder of hand-named lecture notes sorts into nonsense.
   Default should be created-date descending with a toggle. Small, and stated.

3. **Seeing sidenote editing actually run.** The feature is built, tested and
   committed, and no one has watched it work. Wants a real session: create a
   note, type an expression into it, watch it wrap in the margin, Escape out.

4. **External-change detection.** Nothing watches the vault. Edit a note on the
   other machine, save here, and the other version is overwritten silently.
   Currently survivable; becomes a data-loss bug the day sync lands, so it
   belongs *before* git sync rather than after.

5. **Git sync (§8)** — auto-commit on idle and on close, conflict detection on
   open, in-app side-by-side resolution, and keep-both-as-separate-notes when
   resolution fails. Never drop a student into `git mergetool` before a lecture.

6. **PDF export (§10)** — one layout engine for screen and page, which is the
   most constraining line in the spec and is still honoured: `math_layout` is
   pure and headless. Pages, keep-together rules and sidenote repagination do
   not exist yet.

7. **Peek is not separate from open (§7.1).** `tabs.rs` treats preview and peek
   as the same thing; the spec wants peek to never create a tab, which is the
   entire reason both exist. No pinning either.

8. **A display math block should look like one** — centred and at display size,
   rather than inline-sized text on a tinted slab.

9. **Matrices and cases.** **A raw math text node**, deferred by the reader as
   not a problem yet.

Two loose ends that are not features. `code.zip` is tracked in the repo — 243 KB
of binary committed by accident in `44c1251`, wants `git rm --cached` and a
`.gitignore` line. And the §11 performance budget has never been measured;
keystroke-to-glyph under 16 ms and one-frame reflow are the two that would show
at lecture speed, and the brush and sidenote work both added per-frame layout
walks worth timing.

---

## Session log

**`src/markdown.rs`, `src/document/mod.rs`, `src/document/layout.rs`,
`src/theme.rs`** — `####` headings now parse as level-4 headings (`#{1,4}`),
render at 30px with a 17.0 font size. `#####` stays a paragraph.

**`src/theme.rs`** — faux-bold weight cut from `size * 0.045` to `size * 0.025`
(`TextStyle::bold()` and the resize path). Bold text reads as heavier without
the ~1.6px outline growth that looked like a font-size jump.

**`src/document/mod.rs`** — `prune_runs` no longer collapses an emptied
Heading back to a plain Paragraph placeholder; `set_heading` on an empty line
survives and typing into it stays a heading.

**`src/document/mod.rs`** — fixed `insert_text` computing the caret against a
zero-length trash run created by the splice at a run edge. The caret is now
re-derived from the flat `flat + len` *after* `enforce()` prunes, so bold/italic
insert no longer shifts the caret back a whole run.

**`src/shell/input.rs`** — Esc popping a pending B/I style context now routes
through `Tabs::touch` (bumps revision), so the B/I marker disappears the same
frame instead of lingering until the next keystroke.

**`src/vim/mod.rs`, `src/vim/motion.rs`, `src/shell/input.rs`** — new
`Motion::Append` clamps to the current block's end; `a`/`A` no longer jump to
the next line at EOL.

**`src/input.rs`** — Ctrl/Alt/Super + key no longer generates typed text;
Shift alone still types (`SUPPRESS_TEXT_MODS` = CONTROL|ALT|SUPER).

**`src/document/mod.rs`** — `delete_char` (vim `x`) rewritten from run-local
(`o >= run_len` no-op at a run's end) to flat-position based, so it deletes the
char after the caret across a plain→bold run boundary. The caret now eats into
a bold block normally instead of being stranded at the boundary.

New regression tests added alongside each fix.

---

## Session log — badges and highlights

**`src/document/mod.rs`** — `Style` gained `badge` and `highlight` flags plus
`Style::is_boxed()` (true for `code` and `badge`: runs that draw their own box
and so own their whole extent). `toggle_bold`/`toggle_italic` early-return on a
boxed context; `toggle_code`/`toggle_badge` replace the whole pending style
rather than stacking onto it; `toggle_highlight` is ignored on a boxed context.
Badges are a style on a text run, not a new `Inline` variant, so the run's own
text is the label and typing, caret motion, wrapping, deletion and vim motions
all work unchanged.

**`src/document/markdown.rs`** — grammar for `[[BADGE]]` (verbatim label, like
a code span) and `==highlight==`. `==` is the one recursive marker: `parse_inline`
recurses into the body and distributes `highlight` over the returned runs,
skipping boxed ones, so emphasis nests inside a highlight instead of being
swallowed. `serialize_runs` groups adjacent highlighted runs under one `==`
pair — re-opening at every emphasis change both reads badly on disk and parses
back as several marks. Escaping is narrow: only the doubled forms `==` and `[[`
are markers, so only those get escaped.

**`src/document/layout.rs`** — new public `advance(text, block, style, measure)`,
the single seam that puts a badge's box into the flow (adds `BADGE_PAD * 2`).
Used by `piece_width` (wrap) and `x_of_flat` (caret); the left pad is also added
in `x_of_flat`'s prefix branch and in `caret_for_click` either side of the
per-char loop. Fixes badges clipping into the text after them — the box was
originally excluded from the advance on purpose, which was wrong.

**`src/theme.rs`** — `BADGE_SIZE`/`BADGE_INK`/`BADGE_PAD`/`BADGE_HEIGHT` and
`HIGHLIGHT` live here rather than in the editor component, since the document
layout needs the metrics too.

**`src/components/editor.rs`** — badge outlines, the highlight underline bar,
and the glow halo. `spans()` merges adjacent decorated pieces so a mark split
across runs reads as one. The editor mirrors `layout::advance`'s arithmetic on
its own draw cursor, so box, caret and following text cannot drift.

**`src/shell/mod.rs`** — the highlight glow is a bare `Layer` (not a `Region`)
carrying `ShaderEffect::Blur`, composited directly above the editor. `Editor`
holds a cloned handle and clears/clips it itself, so it can never hold a halo
for a bar that is no longer there.

**`src/shell/commands.rs`, `src/tabs.rs`** — `format.badge` and
`format.highlight` commands (auto-appear in the `/` menu), plus caret-only
`Tabs` wrappers that never promote a preview tab.

---

## Session log — memory audit and fixes

Audit first: a 40-line control app (winit window + wgpu surface + device, zero
textures, zero draws) measures **161 MB RSS / 209 MiB VRAM**. That is the
NVIDIA/Vulkan driver floor, and it is most of what the process reports.
Typewritter's own heap is ~19 MB. Two useful facts fell out: `nvidia-smi`
reports VRAM in **256 MiB granules**, so it is a poor benchmark for small
changes; and `FontSystem::new()` costs **1.3 MB, not 100 MB** — fontdb 0.23
drops the mmap after parsing. Full write-up in `~/.dev/Atomos/FEEDBACK.md`.

Measured effect of the three fixes below, idle, release, same window:
VRAM 471 MiB → 215 MiB, RSS 240 MB → 230 MB, idle CPU 8.6% → 2.1%.

**`src/ui.rs`** — `Region::layer` is now `Option<Layer>`, with `detached()`,
`attach()`, `detach()` and `is_attached()`. `update` and `measure_into` no-op
without a layer. Every layer is a full-surface render target whether or not
anything is drawn into it, so a permanently-closed overlay was pure dead VRAM.

**`src/shell/mod.rs`** — the four overlay regions (onboarding, dialog, palette,
slash menu) start detached; `sync_overlay_layers` attaches on open and drops on
close, each frame after input. Dropping the handle is the whole deallocation —
the renderer's stack holds only a `Weak`. Note the behaviour change: a
re-attached layer goes back on *top*, so overlay stacking now follows what was
opened last rather than construction order.

**`src/shell/stepped.rs`** (new) — `Stepped` wraps `Animation`, quantises the
weight into N steps, and `advance()` reports a *step change* instead of "still
running". `Animation::advance` returns `self.playing`, which for `Repeat::Loop`
and `Repeat::PingPong` is permanently true, so the looping caret and pulse meant
`ControlFlow::Wait` was unreachable and the app rendered continuously forever.
`wake_in()` says when the next step is due, since an animation that falls silent
must supply its own wake or it freezes. It keeps its own wall clock rather than
taking the frame `dt`: `FrameScheduler::animation_delta` clamps to 100 ms, and
something that sleeps 525 ms on purpose cannot be driven by clamped time.

**`src/shell/mod.rs`** — caret is `Stepped` with 2 steps (on/off), pulse with 16
(`Topics` already rounds it to sixteenths, so that is every value that reaches
the screen). The pulse had to be converted too, otherwise `animating` never goes
false and the caret change is unmeasurable. Pulse is why idle is 2.1% and not
~0%: it still wakes ~13×/sec. New `Shell::wake_at()`.

**`src/main.rs`** — `about_to_wait` turns `Shell::wake_at()` into a
`ControlFlow::WaitUntil`, but only in place of `Wait`; anything owed now must
not be delayed behind an animation. When the deadline passes it requests the
redraw and clears the wake — without that, `WaitUntil` gets a past instant every
iteration and the loop spins at 100% CPU (it did, first try). Also
`trim_vulkan_drivers()`, which sets `VK_LOADER_DRIVERS_DISABLE` before wgpu
builds its instance: the loader opens every ICD manifest on the machine, not
just the one it uses. Only drivers that can never be the right answer here are
disabled (`lvp`, `virtio`, `gfxstream`, `hasvk`, `asahi`) — worth ~8 MB. An
explicit choice in the environment always wins.

Open, deliberately not done: the highlight glow layer still costs three
full-surface textures for a 2.5px halo (`ponytail:` note at its `set_effect`
call); pausing the pulse when nothing is live would take idle CPU near zero, but
that is a product decision. The larger renderer-side items — rect-sized layer
textures, the measure-only shape cache, `EffectGlyphs` eviction — are Atomos's
and are ranked in `~/.dev/Atomos/FEEDBACK.md`.

---

## Session log — bug fixes from classroom testing

**`src/vault.rs`, `src/components/dialog.rs`, `src/shell/commands.rs`,
`src/shell/input.rs`** — folders. There was no way to create one and no way to
put a note inside one: `create_note` refused any name containing `/` and always
wrote to the vault root. Now `Ctrl+Shift+N` opens a `Prompt::NewFolder`, a new
note lands in whatever folder is selected in the tree (the parent folder when a
file is selected, the vault root when nothing is), and a typed name may be a
relative path such as `math/lecture 3` whose parents are created on the way.
Absolute paths and `..` are refused in `resolve_new_path`. `Vault::refresh` used
to re-read only the top level and throw away every expanded folder, so a file
created inside one was invisible; it now merges the fresh listing with the old
expansion state, and the new `Vault::reveal` expands every ancestor of whatever
was just created.

**`src/components/editor.rs`** — the current-line band was drawn inside
`if self.caret_on`, so the whole line highlight blinked along with the caret.
The band is the line indicator and is now drawn every frame; only the caret bar
stays blink-gated.

**`src/components/editor.rs`, `src/shell/mod.rs`** — the editor could not
scroll past the end of the file, which left the last line stuck against the
bottom edge of the window. New `editor::max_scroll` adds `OVERSCROLL` (half the
visible height) of empty space past the last line, and `Shell::editor_max_scroll`
calls it.

**`src/main.rs`** — random black flashes lasting one to three frames. A
compositor can report a zero-sized window mid-resize; solving the layout against
it collapses every region's rect, and `Region::update` then clears every layer
without drawing into it, leaving the renderer's clear colour on screen.
`App::frame` now returns early on a degenerate viewport, keeping the last good
frame.

**`src/main.rs`, `src/input.rs`** — dead keys and system input methods produced
nothing on macOS. winit only sends `WindowEvent::Ime` when IME is allowed and it
is off by default; its own documentation notes that on macOS IME must be enabled
for dead-key sequences to combine at all. The window now calls
`set_ime_allowed(true)` and `Ime::Commit` folds into the same per-frame `text()`
buffer. winit delivers no `KeyboardInput` during a preedit, so a commit cannot
double up with typed text. Not yet drawn: the preedit itself, so a sequence in
flight is invisible until it commits.

**`src/shell/mod.rs`, `src/shell/input.rs`** — only the file tree could be
drag-resized. `Shell::divider()` became `dividers()`/`divider_at()` over a new
`Divider` enum, and `dragging` went from `bool` to `Option<Divider>`. The topics
outline and the sidenote margin are measured from their own right edge
(`dragged_width`), since they grow leftwards, while the tree still grows
rightwards. `divider_hot` still means the tree's divider specifically, because
that is what the file tree draws its accent rule from.

Every change is covered by `cargo test` (205 passing), and the folder creation,
the non-blinking band and the IME path were additionally verified against the
running app by injecting real key events through `/dev/uinput` and reading the
saved Markdown back off disk — but the two mouse-driven paths (wheel overscroll
and divider dragging) rest on unit tests only, because pointer injection could
not be made to reach the app under this compositor.

---

## Session log — Ctrl brush selection

**`src/shell/input.rs`, `src/shell/mod.rs`, `src/document/layout.rs`,
`src/document/math_layout.rs`, `src/components/editor.rs`** — Ctrl+left-drag
now keeps a persistent set of selected contextual nodes. The 18px radius is
the single `editor::BRUSH_RADIUS` constant. It reuses `ContextHit`, rather than
adding a second document selection model: words, badges, inline/fenced code,
whole empty math atoms, and deepest math nodes all use their existing identity.

The brush circle and its selection overlays are rendered from the current
layout every frame, so scroll and resize cannot stale hit boxes. Circle/rect
intersection is inclusive at tangency. Structural descendants win over their
parents; colliding siblings are all selected in paint order.

One brush stroke toggles a target only when it is entered. The pointer path is
sampled at no more than one brush radius apart, so a fast drag cannot skip a
node. The current circle alone defines the inside set, which permits a later
leave/re-enter to toggle again. Ctrl-click retains unrelated selections; Esc
clears the brush set and still reaches any open popup on that same keypress.
Dialogs retain mouse priority. Code blocks are hit-tested across their full
rendered slab, not merely their text width.

Tests cover circle tangency, node precedence, mixed/deduplicated targets,
full-width code blocks, entry/re-entry toggling, sweep sampling, and Esc reset.
Final verification: fmt, warnings-denied Clippy, and 448 tests passed. Renderer
smoke reached the expected headless Wayland `NoCompositor` boundary before any
renderer work; the brush SVG ring has a parser test.
