---
title: Command line
nav: CLI
description: Convert, render, replay, harvest and manage fonts and versions without opening the editor.
order: 24
glyph: "$"
---

## Opening files

```sh
acidtrip                     # new canvas
acidtrip logo.ans more.xb    # each file opens in its own tab
```

acidtrip opens `.ans` `.xb` `.bin` `.adf` `.idf` `.tnd` `.pcb` `.avt` `.asc` and `.acid` files, or a new file name.

## convert and render

`acidtrip convert IN OUT` converts between any formats. The output format comes from the extension unless you pass `--format`.

| Flag | What it does |
|---|---|
| `--format F` | output format, instead of the extension |
| `--scale N` | pixel scale for PNG and GIF (default 1) |
| `--gif MODE` | `still`, `reveal`, `layers`, `frames` or `auto` (frames when the piece is animated) |
| `--baud N` | baud rate for reveal animations (default 14400) |
| `--line-length N` | ANSI: wrap lines at N characters for BBSes |
| `--no-sauce` | don't attach a SAUCE record |
| `--pixel-exact` | SVG with pixel-exact bitmap glyphs |
| `--identifier NAME` | name for C/Pascal/ASM arrays and React components (default `AcidArt`) |

`acidtrip render IN OUT.png --scale N` is a shortcut for converting to PNG.

```sh
acidtrip convert art.ans art.svg --pixel-exact
acidtrip convert art.ans art.gif --gif reveal --baud 9600
acidtrip render art.xb art.png --scale 2
```

## replay

`acidtrip replay IN.acid OUT` writes a `.gif` or an asciinema `.cast` of how the piece was drawn. Only `.acid` keeps the edit history.

| Flag | What it does |
|---|---|
| `--fit SECS` | fit the whole replay into SECS seconds (default 30) |
| `--speed N` | play N times as fast as it was drawn, instead of `--fit` |
| `--scale N` | GIF pixel scale |
| `--keep-idle` | play long pauses in real time instead of cutting them to a second |
| `--hide-undone` | leave out work that was undone |

## fonts and harvest

```sh
acidtrip fonts list                 # installed fonts
acidtrip fonts get                  # download the TheDraw font packs
acidtrip fonts show FONT "TEXT"     # render text to the terminal
acidtrip fonts install FILE         # add a .tdf, .flf or .zip
```

`acidtrip harvest SOURCE` builds fonts and stencils from a 16colo.rs pack (`16colo.rs:acid-50`), a URL, a file or a folder. `--list` only lists the logos it finds, `--stencils-only` saves them all as stencils, and `--complete` has the AI draw the missing letters. See [Fonts and stencils](/docs/fonts-stencils/).

## mcp

`acidtrip mcp` is a stdio MCP server for Claude Code. It attaches to a running editor if there is one; `--session PID` picks which, and `--headless` ignores running editors and works on a private canvas. See [AI](/docs/ai/).

## versions, paths and keys

- `acidtrip versions FILE` lists a document's saved versions. `--restore HASH --out OUT` restores one into a file; a hash prefix is enough.
- `acidtrip paths` prints where the config, fonts, stencils, versions, recovery files and sockets live.
- `acidtrip keys` lists every action id with its title and keys, for [keymap overrides](/docs/keymaps-config/).

The `ACIDTRIP_RUN` environment variable takes a comma-separated list of action ids to run at startup.
