# baca

`baca` is a native, read-only Markdown reader. Its name is Indonesian for
“read.” It gives a folder of Markdown files an editorial, distraction-free
reading surface.

## Run

```bash
cargo run -- path/to/notes.md     # open one document
cargo run -- path/to/folder       # open a library
```

Passing a file opens that document and treats its folder as the library.
Passing nothing opens an empty shelf; press `Ctrl+O` to choose a folder.

## Controls

- `Ctrl+O` — choose a Markdown library folder
- `Ctrl+R` — reload the library and the open document from disk
- `Ctrl+T` — switch theme: Paper, Kraft, Malleable
- `Ctrl+A` / `Ctrl+C` — select the whole document, copy the selection
- `Alt+Left` — back to the shelf

Drag across the page to select text. Click a link to follow it: external URLs
open in the browser, and relative or `[[wiki]]` links open in place.

## What it renders

Headings, paragraphs, bold, italic, strikethrough, inline code, links,
blockquotes, ordered and nested lists, task lists, tables, images, footnotes,
horizontal rules, and fenced code blocks with syntax highlighting.

baca is read-only by design: it never writes to the Markdown it opens.

## Typography

baca prefers Fraunces, Newsreader and IBM Plex Mono, and falls back to the
closest installed family. Install those three for the intended look — gpui
cannot synthesise bold or italic for a family that is missing.

## Development

```bash
cargo test    # parser coverage
cargo build
```

See `FEATURES.md` for what is done and what is planned.
