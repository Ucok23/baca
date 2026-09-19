# baca

`baca` is a native, read-only Markdown reader. Its name is Indonesian for
“read.” It opens one Markdown file at a time and gives it an editorial,
distraction-free reading surface.

## Run

```bash
cargo run -- path/to/notes.md
```

If no file is supplied, baca opens a built-in guide so the interface remains
useful on first launch.

## Controls

- `Ctrl+T` — switch theme: Paper, Kraft, Malleable
- `Ctrl+R` — reload the file from disk
- `Ctrl+0` — scroll to the top

The first release is intentionally read-only: it never writes to the opened
Markdown file.

