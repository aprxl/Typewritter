# Typewritter Studio

An alternative native interface, developed on `codex/studio-ui`. The experiment replaces the previous visual system rather than adding a second set of widgets. `master` is unchanged.

## Direction

The reference is a personal writing instrument: a quiet place for serious mathematics, with enough warmth to feel owned. The first steel-blue palette was too anonymous. This version pairs chalk paper with lavender surroundings and coral gestures; the dark appearance uses mulberry surfaces with apricot accents. A handwritten tau gives the application its own mark.

The visual hierarchy has three levels:

- **Workspace:** a compact toolbar, inset tabs, restrained path labels, and a quiet file tree. Navigation uses small sans-serif type so it recedes behind the document.
- **Document:** a rounded sheet, a centered reading measure, generous heading spacing, and annotation markers instead of a continuous margin divider. The annotation column only occupies space when the document has sidenotes.
- **Actions:** floating cards with native gradients, blurred shadows, clear keyboard hints, and short entrances. Selection and hover remain legible in both appearances.

Inter is bundled under the SIL Open Font License for interface and prose. Mathematics retains JuliaMono for its symbol coverage; code and small numeric labels use the system monospace. This pass changes screen and exported prose typography together, so their measurements continue to agree.

## Try it

```powershell
git switch codex/studio-ui
cargo run
```

- Press **Space** in Normal mode to open the command palette.
- Run **Switch appearance**, or click the toolbar's sun/moon control, to move between Chalk and Mulberry. The existing GPU snapshot transition reveals the new appearance from the switch.
- Press **Ctrl+Shift+C**, run **Toggle focus mode**, or click **Focus** to collapse the auxiliary panels. The reading measure stays centered; the toolbar indicates when focus mode is active.
- Use **Open file** or the toolbar search to find notes. A compact window shows one results column; a wider window adds a preview. Results work with keyboard selection or a click, and opening a note reveals it in the tree.
- Use the **+** tab control or **Ctrl+N** to create a note. The empty editor offers the same new/open actions.
- Long tabs elide their labels and page through whole tabs. The active tab remains visible. Long outlines scroll independently; deep breadcrumb paths preserve their tail.
- Render statistics and row hit bands remain available through the command palette, with statistics hidden by default.

## Native implementation

No browser surface or bitmap mockup is involved. The tau is a vector path; surfaces use Atomos gradients and rounded geometry; popup depth uses its blur layer. Existing theme transitions, menu springs, and manual region invalidation remain in use. The old decorative writing pulse and unused modal slide state were removed.

`theme.rs` owns the palette, font sources, mark, and shared painting. `document_surface.rs` owns the sheet, `empty_state.rs` owns the empty editor, and each navigation or overlay component owns its own geometry. The shell routes controls through the command table. The editor's centered origin is shared by painting, hit testing, caret placement, and popup anchors.

The interaction pass also fixes scrolled finder rows being painted at the wrong offsets, centered modal shadows being painted around an empty viewport, and hidden panels continuing to receive pointer input.

## Verification

Run the repository checks after changes:

```powershell
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

The Windows verification includes live native rendering in both appearances, the editor with math and sidenotes, command palette, finder, focus mode, and resizing between 620×520 and 1600×1000. The native runtime log is checked for panics and wgpu validation failures. Geometry tests cover centered text, compact finder bounds, scrolled row targeting, tab overflow, and empty-state actions.

macOS and Linux runtime rendering have not been exercised in this Windows session. Theme choice remains session-only, as before this experiment.
