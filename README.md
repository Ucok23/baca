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

Launched with no arguments, baca reopens whatever you were last reading, at
the place you left it.

## Controls

| | |
|---|---|
| `Ctrl+O` | choose a Markdown library folder |
| `Ctrl+F` | search — the open document, or the shelf |
| `n` / `N` | next and previous match |
| `Ctrl+B` | show or hide the outline |
| `Ctrl+R` | reload from disk |
| `Ctrl+T` | switch theme: Paper, Kraft, Malleable |
| `Ctrl` `+` `-` `0` | text size |
| `Ctrl+A` / `Ctrl+C` | select all, copy the selection |
| `j` `k` `space` `g` `G` | move through the page |
| `↑` `↓` `Enter` | move through the shelf, open |
| `Esc` / `Alt+Left` | back to the shelf |

Drag across the page to select text. Click a link to follow it: external URLs
open in the browser, and relative or `[[wiki]]` links open in place. Click an
outline entry to jump to that heading.

The open document reloads by itself when it changes on disk.

## What it renders

Headings, paragraphs, bold, italic, strikethrough, inline code, links,
blockquotes, ordered and nested lists, task lists, tables, images, footnotes,
horizontal rules, and fenced code blocks with syntax highlighting.

YAML front matter is read as metadata rather than shown as content: it
supplies the title, and its tags appear in the sidebar.

baca is read-only by design: it never writes to the Markdown it opens.

## Typography

baca prefers Fraunces, Newsreader and IBM Plex Mono, and falls back to the
closest installed family. Install those three for the intended look — gpui
cannot synthesise bold or italic for a family that is missing.

The theme follows the desktop's light/dark preference until you pick one with
`Ctrl+T`, after which your choice sticks.

## Settings

Remembered in `~/.config/baca/settings.json`. Deleting it resets baca to
defaults; nothing there is required for it to run.

## Development

```bash
cargo test    # parser coverage
cargo build
```

See `FEATURES.md` for what is done and what is planned.
