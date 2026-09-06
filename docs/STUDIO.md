# Typewritter Studio

An alternative native interface, developed on `codex/studio-ui`. The experiment replaces the previous visual system rather than adding a second set of widgets. `master` is unchanged.

## Completion review — 6 September 2026

The original interface overhaul is implemented. This document is the retained design record; there is no separate unfinished redesign plan. The review includes the subsequent notation and symbol-inspector commits (`b4cddea`, `cc17502`, `816e8a8`).

| Area | Delivered |
| --- | --- |
| Visual identity | Chalk/lavender/coral and mulberry/apricot palettes, bundled Inter, vector tau mark, shared native surfaces |
| Workspace | Toolbar, tabs, file tree, breadcrumbs, status, search and panel controls, compact navigation |
| Writing | Centered document sheet, heading hierarchy, empty-state actions, anchored sidenotes and outline |
| Actions | Command palette, finder, slash and math menus, symbol inspector, format bar, dialogs and onboarding |
| Focus | Centered active visual line, GPU fading of surrounding content, restoration of the previous panels |
| Rendering | Native gradients, blurred popup shadows, transitions, corrected bold outlines, shared screen/PDF notation painting |

The completion audit found and closed two compact-window gaps: the slash menu could flip above the screen, and a long math completion list could panic while placing its card. Math interpretations now use a six-row window with a visible match count; arrows, Ctrl+N/P and the wheel reach the remaining choices while variants stay visible. Painting and hit testing share the scroll offset. Variant selection paints above its cell surface, and dismissal retains the visible rows without computing geometry against an empty viewport.

The final audit passes formatting, Clippy with warnings denied, and 743 tests. Windows native checks cover the previously crashing `s` completion at 620×484, scrolling to later interpretations and variant cells, acceptance and dismissal, the compact slash menu, and the full workspace at 1600×1000. Both appearances were inspected using an isolated scratch vault. The final runtime log contains no panic or wgpu validation error. The platform and persistence limits at the end of this document still apply.

## Direction

The reference is a personal writing instrument: a quiet place for serious mathematics, with enough warmth to feel owned. The first steel-blue palette was too anonymous. This version pairs chalk paper with lavender surroundings and coral gestures; the dark appearance uses mulberry surfaces with apricot accents. A handwritten tau gives the application its own mark.

The visual hierarchy has three levels:

- **Workspace:** a compact toolbar, inset tabs, restrained path labels, and a quiet file tree. Navigation uses small sans-serif type so it recedes behind the document.
- **Document:** a rounded sheet, a centered reading measure, generous heading spacing, and annotation markers instead of a continuous margin divider. The annotation column only occupies space when the document has sidenotes.
- **Actions:** floating cards with native gradients, blurred shadows, clear keyboard hints, and short entrances. Selection and hover remain legible in both appearances.

Inter is bundled under the SIL Open Font License for interface and prose. Mathematics retains JuliaMono for its symbol coverage; code and small numeric labels use the system monospace. This pass changes screen and exported prose typography together, so their measurements continue to agree.

## Notation

An expression used to be prose with a highlight behind it. It now has a typography of its own, on three axes that never say the same thing twice.

**Identity is colour.** Every symbol carries a hue, so `x` and `y` differ the way two words differ rather than only by position. The default comes from where a letter sits in its own alphabet, not from a hash of it, so the letters you actually reach for together — `x y z`, `i j k`, `m n` — cannot collide; the Greek alphabets are offset so an alpha is never an `x`. Ten hues cannot uniquely colour a hundred symbols, and the per-symbol override is what the rest is for.

**Role is shape.** A variable is a filled wash, a constant an outline, a function both. An outline is never a colour of its own: it is its fill, rotated in HSL away from the page and slightly up in chroma, so a border is always a shade of the thing it borders. Because the distinction is a shape and not a hue, it survives greyscale, a projector, and a printed page.

**Grammar recedes.** Operators, relations, punctuation, large operators and delimiters are set in a quieter ink than the terms they join. The summation sign is optically narrowed and lifted to match the weight and axis of the integral family. Numerals get an ink of their own — a quantity is not a name — and long ones group in threes outward from the decimal point, ISO 31-0 style, so `299792458` reads as `299 792 458`. The separator is an advance rather than a node: the caret cannot stop in it and backspace has nothing to delete.

Both axes are per symbol and both are optional. Overrides live in `~/.typewritter/config.toml` rather than in the documents, because a symbol's colour is a fact about your vocabulary and not about one note:

```toml
[math.symbols.epsilon]
hue = "coral"
shape = "outline"
```

Clicking a symbol in Normal mode opens the **symbol inspector**, which replaces the old list of words for choices whose whole content is how they look. A head shows the symbol as it stands and the identity its styling is keyed on. Role rows carry their shapes; colour is a ten-swatch palette (a swatch shows its hue whole, fill and edge, because ten hairlines are not a palette); highlight is three chips of the same symbol; variants show the letterforms themselves. Every cell draws the outcome of choosing it, including "back to automatic". The card speaks the same popup vocabulary as the in-math completion card, and choices land in the config immediately.

Painting is shared with PDF export, so a page carries the same hues, shapes and inks the screen does.

## Try it

```powershell
git switch codex/studio-ui
cargo run
```

- Press **Space** in Normal mode to open the command palette.
- Run **Switch appearance**, or click the toolbar's sun/moon control, to move between Chalk and Mulberry. The existing GPU snapshot transition reveals the new appearance from the switch.
- Press **Ctrl+Shift+C**, run **Toggle focus mode**, or click **Focus** for a typewriter view. The active visual line sits at the window's vertical center with full contrast; surrounding prose, equations, and annotations recede. The first and last lines can both reach the center. Wheel scrolling still lets you browse; moving the caret or typing brings the active line back. Leaving Focus restores the previous panel arrangement. Opening a sidenote leaves Focus so you can see what you are editing.
- Use **Open file** or the toolbar search to find notes. A compact window shows one results column; a wider window adds a preview. Results work with keyboard selection or a click, and opening a note reveals it in the tree.
- Use the **+** tab control or **Ctrl+N** to create a note. The empty editor offers the same new/open actions.
- Long tabs elide their labels and page through whole tabs. The active tab remains visible. Long outlines scroll independently; deep breadcrumb paths preserve their tail.
- Render statistics and row hit bands remain available through the command palette, with statistics hidden by default.
- Click a symbol inside an expression in **Normal** mode to open the symbol inspector. Pick a hue or a highlight shape and every occurrence in the vault follows; **Back to automatic** drops the override again.

## Native implementation

No browser surface or bitmap mockup is involved. The tau is a vector path; surfaces use Atomos gradients and rounded geometry; popup depth uses its blur layer. Existing theme transitions, menu springs, and manual region invalidation remain in use. The old decorative writing pulse and unused modal slide state were removed.

`theme.rs` owns the palette, font sources, mark, and shared painting. `document_surface.rs` owns the sheet, `empty_state.rs` owns the empty editor, and each navigation or overlay component owns its own geometry. The shell routes controls through the command table. The editor's centered origin is shared by painting, hit testing, caret placement, and popup anchors.

Notation splits the same way. `math_style.rs` holds the semantics — which hue an identity falls on, which shape a role asks for, and what the reader has overridden — while `theme.rs` holds the ten-hue ramp, the two notation inks, and the HSL shade that derives a border from its fill. Math layout resolves a symbol's style as it measures, so the style server carries a revision into the document layout cache and a change flag into the frame's invalidation, exactly as the palette does. `symbol_menu.rs` is a new component; it shares the context menu's region and reveal clock, and which of the two faces is showing follows from the target rather than from stored state.

The interaction pass also fixes scrolled finder rows being painted at the wrong offsets, centered modal shadows being painted around an empty viewport, and hidden panels continuing to receive pointer input.

Focus uses a GPU opacity band on the editor's own layer, with a short fade and soft edges. The band follows the same wrapped-line bounds used by the caret and scroll camera. Its uniforms update in place, and the effect releases its output texture outside Focus. The document surface remains fully opaque beneath it.

Synthetic bold now uses filled and stroked outlines. Swash's previous dilation could shrink Inter's `f`; the replacement preserves the letter and matches PDF export's construction. Regression coverage checks ink growth, counters, and baseline placement at five sizes.

## Verification

Run the repository checks after changes:

```powershell
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

The Windows verification includes live native rendering in both appearances, the editor with math and sidenotes, command palette, finder, focus mode, and resizing between 620×520 and 1600×1000. The native runtime log is checked for panics and wgpu validation failures. Geometry tests cover centered text, compact finder bounds, scrolled row targeting, tab overflow, and empty-state actions.

The notation pass was checked the same way: an expression with ten letters, both Greek cases, all three roles, grouped numerals and a full set of operators, read in both appearances; the inspector opened over a constant, a hue picked and seen to reach every occurrence and `config.toml`, then reset; and the note exported to PDF from a debug build, where a highlight outline that failed to parse would assert rather than pass quietly.

macOS and Linux runtime rendering have not been exercised in this Windows session. Theme choice remains session-only, as before this experiment.
