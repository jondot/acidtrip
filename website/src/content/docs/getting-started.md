---
title: Getting started
nav: Start
description: Install acidtrip, open a file, and find your way around the screen.
order: 1
glyph: "⌂"
---

## Install

One line installs the binary for macOS or Linux, Intel or Arm:

```sh
curl -fsSL https://raw.githubusercontent.com/jondot/acidtrip/main/install.sh | sh
```

It puts `acidtrip` in `/usr/local/bin`, or `~/.local/bin` when that isn't writable. Add `-s -- --version v0.1.0` after `sh` to pick a release. The archives are also on the [releases page](https://github.com/jondot/acidtrip/releases/latest), signed with cosign.

To build from source instead, with Rust installed:

```sh
cargo install --git https://github.com/jondot/acidtrip acidtrip
```

## First run

```sh
acidtrip                 # new canvas
acidtrip logo.ans        # open (or create) a file
```

Give it any number of files and each opens in its own tab. acidtrip opens `.ans` `.xb` `.bin` `.adf` `.idf` `.tnd` `.pcb` `.avt` `.asc` and its own `.acid` files. A name that doesn't exist yet opens as a new, empty document with that name.

The first run also writes a config file with comments, which you can edit later. See [Keymaps and config](/docs/keymaps-config/).

![acidtrip with the sunset piece open, the sidebar on the right and the status bar below](/shots/overview.png "The editor: the canvas, the sidebar panels, and the status bar.")

## Terminals

A terminal with truecolor and mouse input is enough: iTerm2, kitty, WezTerm, Ghostty, Alacritty, or Terminal.app with fewer key combinations. Colors always come from the document's own VGA palette, so the art looks the same whatever your terminal theme.

Terminals that report exact mouse pixels (kitty, WezTerm, Ghostty, xterm and others) let the [smart pen](/docs/brushes/) follow the pointer inside a cell. Terminals with graphics (iTerm2, kitty, sixel) also get a pixel-exact preview, which `Alt-W` toggles.

## The screen

- **The canvas** takes the left side. Drag with the mouse to paint.
- **The sidebar** is a stack of panels: TOOLS, the active tool's options, COLORS, CHARACTERS, LAYERS, GALLERY and the MAP. Every option shows all its choices as chips you click, so nothing hides behind a key. <kbd>Ctrl-B</kbd> hides and shows it.
- **The status bar** runs along the bottom. Hover over anything, in the sidebar or the status bar, and it says what that thing does and its key.

The status bar is clickable too. The canvas size (`80x25`) opens *Canvas size*. `CLASSIC` / `MODERN` switches the document kind, and `iCE` toggles iCE colors. <kbd>Ctrl-Z</kbd> undoes any of them.

Everything works with one mouse button. The right button is only ever a shortcut.

## Commands and keys

<kbd>Ctrl-K</kbd> (or <kbd>Ctrl-P</kbd>) opens the command palette. It fuzzy-searches every command in acidtrip and shows each one's key, so it is the quickest way to find anything.

![The command palette listing commands with their keys](/shots/palette.png "Ctrl-K: every command, searchable.")

<kbd>?</kbd> shows the live key sheet, the keys as they are bound right now, including your own overrides. <kbd>Alt-H</kbd> opens it from anywhere.

## Classic and Modern documents

- **Classic:** CP437 characters and 16 colors, saved losslessly to `.ans`, `.xb` and `.bin`. This is the scene's format.
- **Modern:** any Unicode character and 24-bit color, for SVG, HTML, React and UTF-8 output.

Click `CLASSIC` / `MODERN` in the status bar to switch, or use *Convert to Modern (Unicode + RGB)* and *Convert to Classic (CP437 + 16 colors)* in the palette. New documents are Classic, 80×25, with iCE colors on; the `[new_doc]` section of the config changes that.

## Canvas size and SAUCE

Widths are anything you like: 80, 160 or more. Click the size in the status bar (or palette › *Canvas size…*) for presets, *fit art*, and a preview that shows in red any art a smaller size would cut off. The art stays at the top-left.

![The Canvas size dialog with presets and a preview](/shots/canvas-size.png "Canvas size: presets, fit art, and a preview of what would be cut.")

<kbd>Alt-D</kbd> edits the SAUCE record: title, author, group and the rest. <kbd>Ctrl-D</kbd> opens *Document properties…*.

![The SAUCE dialog](/shots/sauce.png "SAUCE: the credits that travel with the file.")
