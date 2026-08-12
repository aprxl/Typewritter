# Task 03 — Faux-italic in the renderer

**Wave 1, parallel with task 01.** You own: `src/renderer/font_parameters.rs`,
`src/renderer/glyph_effects.rs`, `src/renderer/text_stack.rs`,
`src/theme.rs`. Nothing else.

This edits the **vendored Atomos copies in this repo**, which AGENTS.md
permits ("Changes to the platform happen in this repo, on these copies").
Never touch `~/.dev/Atomos`.

**Read first**: `src/renderer/glyph_effects.rs` (the synthetic-effect
pipeline you are extending), `src/renderer/font_parameters.rs`,
`src/theme.rs` (`TextStyle`), `docs/atomos/RENDERER.md` if the
`FontParameters` docs live there.

---

## 1. Why

Only Georgia Regular is installed — no italic cut, no bold cut. Bold is
already synthesized: `GlyphEffect.embolden` → swash `Render::embolden`,
with the advance correction (`2 * embolden`) applied in both placement and
measurement. Italic joins as a third synthetic effect: a **shear
transform** at rasterization time.

## 2. Changes

1. **`FontParameters`** gains `pub slant: f32` (shear factor; `0.0` =
   none). Update every construction site — grep for `FontParameters`
   literals (`src/theme.rs` builds one in `TextStyle::parameters()`).

2. **`TextStyle`** (`src/theme.rs`) gains `pub slant: f32` (default `0.0`)
   and:

   ```rust
   /// Faux italic: a 12° shear. Georgia ships no italic cut, so — like
   /// faux bold — the slant is synthesized by the renderer.
   pub fn italic(mut self) -> Self { self.slant = 0.21; self }
   ```

   `TextStyle::parameters()` passes it through. `TextStyle::size()` must
   preserve `slant` the way it preserves `weight` (field copy — check the
   existing `bold` handling for the pattern).

3. **`GlyphEffect`** gains `pub slant: f32`; `NEUTRAL` has `slant: 0.0`;
   `From<&FontParameters>` maps it.

4. **`EffectGlyphKey`** gains `slant_bits: u32` so differently-slanted
   rasterizations don't share a cache entry.

5. **`rasterize_glyph`** (`glyph_effects.rs`): compose condense and slant
   into one `swash::zeno::Transform`:

   ```rust
   // x' = condense * x + slant * y ; y' = y
   // (font outlines are y-up with baseline at 0, so +y shears right —
   //  the italic direction)
   let transform = (condense != 1.0 || slant != 0.0)
       .then(|| Transform::new(condense, 0.0, slant, 1.0, 0.0, 0.0));
   ```

   Verify swash recomputes the image bounds after the transform (the
   condense path already relies on this). If a slanted glyph's ink is
   clipped on the right (test with `f`, `j`, `y` at 17.5px), pad the
   placement; check how `layout_to_custom_glyphs_with` uses
   `image.placement` before assuming.

6. **Measurement**: slant adds **no advance delta** —
   `effect_advance_delta` stays as-is. Italic ink overhangs its advance
   slightly at the top right; that is standard faux-italic behavior and
   keeps rendered and measured widths in agreement (the caret/click math
   depends on that agreement).

7. **Shape cache**: `text_stack.rs` keys shaped buffers by
   `(text, font, font_parameters)` (see `shaped_text_key`). Make sure the
   new `slant` field participates in the key exactly the way `weight`/
   `tracking` do today (likely `to_bits`); a stale shape entry shared
   between slanted and roman spans would corrupt measurement.

## 3. Tests

Add a unit test in `glyph_effects.rs` (or wherever the existing tests
live): rasterizing the same glyph with `slant: 0.0` vs `slant: 0.25`
produces a different image, and the slanted one's placement is wider or
shifted right. If rasterization isn't reachable from a test without a GPU,
assert the key separates (`slant_bits` differ) and the transform math is
what you claim — and rely on the smoke test below.

## 4. Verification

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
timeout 6 cargo run > /tmp/typewritter_run.log 2>&1   # renderer changed
```

Exit 124/143 = clean; grep the log for `panic|wgpu error`.

To see the slant working before the document milestone lands, temporarily
chain `.italic()` onto a visible `TextStyle` (e.g. the editor placeholder
text), smoke-run, look, then revert. Leave no test scaffolding behind.

## 5. Non-goals

- No new fonts, no font loading changes.
- No per-glyph positioning change for italic beyond the shear (no advance
  or tracking correction).
- Nothing in `src/document/` — the model doesn't know what a pixel is.
