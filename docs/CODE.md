# Code highlighting

In Normal mode, click a code block or inline code span to choose a language.
Fences with a recognized language name highlight automatically on opening.
Inline code starts plain until a language is chosen. Unknown fence languages
stay plain; no language detection guesses are made for short snippets.

Bundled languages: C, C++, Rust, Lua, Python, JavaScript, TypeScript, Java, C#,
and Go. Fence aliases include `cc`/`cxx`/`c++`, `rs`, `py`, `js`, `ts`,
`cs`/`c#`/`c_sharp`, and `golang`.

Choose **DIY colors** to disable automatic highlighting for that entire fence
or inline span. Click a word or punctuation mark and choose a color. Hold Ctrl
and drag to brush across multiple tokens; releasing the gesture opens the
color picker. A second stroke can extend the selection. **Clear color** removes
the selected tokens' ink; Escape dismisses the picker and clears the selection.
Colors apply to the selected occurrences, not every occurrence of a word.

The picker shows theme-aware swatches. **Language and mode…** returns to the
language choices; choosing a language resumes automatic highlighting. Switching
between automatic and DIY preserves the manually assigned colors. Click space
inside code to reach its settings directly, including in an empty fence.

DIY annotations and inline language choices are saved inside the Markdown file
in a trailing `typewritter-code` HTML comment. There are no sidecar files or
global color assignments. The comment contains JSON with character ranges and
the corresponding block text, so a mismatched annotation after an external edit
is ignored. Code text stays verbatim inside its usual backticks. The annotations
move with text edits and participate in undo/redo; one brush-color operation is
one undo step. Colors also appear in PDF exports.

Automatic highlighting uses the bundled Tree-sitter grammars and highlight
queries. Whole fences are parsed together so multiline strings and comments
retain their context. This is syntax highlighting, without language servers,
symbol resolution, or cross-language highlighting inside embedded strings.
