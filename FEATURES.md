# baca — Required Features

Tracking list for turning `baca` into a first-class native Markdown reader.
Checked items are implemented and verified in the running app.

Priorities:

- **P0** — without these it is not really a Markdown reader.
- **P1** — the difference between "renders Markdown" and "pleasant to read in".
- **P2** — what makes it the best rather than merely sufficient.

## P0 — Core correctness

All done. Parser coverage is pinned by the tests in `src/markdown.rs`; the
rendering was checked against `src/markdown.rs`'s test document in the running
app.

- [x] **Inline formatting** — bold, italic, inline code, strikethrough, links.
      Requires `Block` to carry `Vec<Inline>` instead of `String`.
- [x] **Clickable links** — external URLs open in the browser, relative `.md`
      links navigate in-app, `[[wikilinks]]` resolve against the library.
- [x] **Tables** — `Options::ENABLE_TABLES` plus a `Block::Table` variant.
- [x] **Task lists** — `Options::ENABLE_TASKLISTS`, rendered as checkboxes.
- [x] **Footnotes** — `Options::ENABLE_FOOTNOTES`, rendered at the document foot.
- [x] **Images** — `Tag::Image` resolved relative to the open file.
- [x] **Syntax highlighting** — the language from `CodeBlockKind::Fenced` is
      currently discarded. Uses syntect, with a theme chosen per palette.
- [x] **Text selection and copy** — drag to select across blocks, `Ctrl+A`,
      `Ctrl+C`. The keyboard path is verified end to end; drag selection shares
      the same machinery but has not been exercised with a real pointer.
- [x] **Nested and ordered lists** — `depth` and `ordinal` are already parsed
      but ignored at render time.

## P1 — Reader-grade

All done. Verified in the running app: search, keyboard navigation, zoom,
front matter, auto-reload on an external edit, and a full session restored
from a launch with no arguments.

- [x] **Outline / table of contents panel** — `Document::outline` is already
      populated and never rendered.
- [x] **Search** — `Ctrl+F`. In a document it finds and steps through matches
      (`n` / `N`); on the shelf it filters the library by title, path and body.
- [x] **Keyboard navigation** — `j`/`k`, space, `g`/`G`, arrows in the library.
- [x] **Persistence** — root folder, theme, last file, zoom, outline state and
      reading position survive a restart. Written on change, every few seconds
      while reading, and on quit.
- [x] **File watching** — auto-reload when the file changes on disk.
- [x] **Text size control** — `Ctrl` `+` / `-` / `0`.
- [x] **Follow the system colour scheme** — Kraft and Malleable are already
      dark; nothing picks between them and Paper automatically.
- [x] **YAML frontmatter** — parsed as metadata. Supplies the title, shows its
      tags in the sidebar, and stays out of the body and the shelf preview.

## Reading experience

- [x] **A home gallery** — baca opens on what you were reading, what you read
      lately, and the collections it found, rather than dropping you straight
      into a document.
- [x] **Recent reads** — the last thirty documents, with their collection, how
      far through you got, and when.
- [x] **Collection discovery** — the usual note folders under `$HOME`, plus any
      you add. Nesting is kept; noise directories are never walked.
- [x] **Onboarding** — an empty gallery says where baca looked and offers to
      be pointed somewhere else.
- [x] **The sidebar follows the page** — it keyed off whether a document had
      ever been opened rather than what is on screen, so stepping back to a
      collection left the previous document's outline sitting there. Clicking
      the wordmark now returns to the gallery from anywhere.
- [ ] **Pinned collections** — keep the ones you use at the top.
- [ ] **Per-collection sort** — by title, by date, by how recently read.

## P2 — Differentiators

- [ ] **Math** — LaTeX / KaTeX-style rendering.
- [ ] **Mermaid diagrams.**
- [ ] **Backlinks and tag index** — for note-vault use.
- [ ] **Editorial typography** — measure control, hyphenation, reading progress.
- [ ] **Focus mode and presentation mode.**
- [ ] **Export** — PDF and standalone HTML.
- [ ] **Tabs or multiple windows.**

## Known bugs

- [x] `reload()` did not re-parse the open document, only rescanned the
      library, so `Ctrl+R` while reading did nothing visible.
- [x] The README promised `cargo run -- path/to/notes.md`, but `main()`
      filtered on `is_dir()`, so passing a file path opened an empty library
      with no error. A file argument now opens that file, with its folder as
      the library.
- [x] The window carried no app id and no title, so a compositor could not tell
      baca's windows from anything else.
- [x] The named typefaces were not necessarily installed, and gpui's fallback
      for a missing family silently drops weight and slant — bold and italic
      stopped rendering. The font is now resolved against what is installed.
- [x] The derived document title was rendered above the document's own opening
      heading, showing the title twice.
- [x] `scan()` read every `.md` file in full on the main thread. It now runs on
      a background thread, and the text it reads is kept for full-text search
      rather than thrown away.
- [x] `render()` cloned the whole `entries` vector every frame. Filtering now
      yields indices instead.
- [x] `Ctrl+R` returned to the top of the document instead of holding the
      reading position.
- [x] List items, table cells and footnote bodies never wrapped: each is a
      flex row whose text child took its automatic minimum width from its
      min-content size, which for a run of text is the whole line, so the item
      could not shrink and the text ran off the page. Visible only once the
      column hit its maximum width, which is why a narrow window looked fine.
- [x] A file changing on disk called `read()`, which forces the reading view,
      so an edit while you were on a shelf yanked you into the document.
- [ ] The document is still not virtualized: every block is laid out each
      frame, whether or not it is on screen.
- [ ] `scan()` holds each file's whole text in memory to make the library
      searchable. Fine for a normal vault, wasteful for a huge one.
