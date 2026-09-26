---
title: Keymaps and config
nav: Config
description: The modern and ACiDDraw key presets, your own bindings, and every setting in the config file.
order: 23
glyph: "⚙"
---

## Two presets

- **modern** is the default: <kbd>Ctrl-S</kbd> saves, <kbd>Ctrl-Z</kbd> undoes, and so on.
- **acid** gives ACiDDraw's Alt-letter keys on top of it.

The `acid` preset changes these keys:

| Key | Action |
|---|---|
| <kbd>Alt-B</kbd> | block menu |
| <kbd>Alt-R</kbd> | undo |
| <kbd>Alt-S</kbd> | save |
| <kbd>Alt-L</kbd> | open |
| <kbd>Alt-D</kbd> | SAUCE info |
| <kbd>Alt-O</kbd> | document properties |
| <kbd>Alt-C</kbd> | clear layer |
| <kbd>Alt-X</kbd> | quit |
| <kbd>Alt-H</kbd> | help |
| <kbd>Alt-Z</kbd> | iCE colors |
| <kbd>Alt-U</kbd> | pick up colors |
| <kbd>Alt-I</kbd> / <kbd>Ctrl-Y</kbd> | insert / delete line |
| <kbd>Ctrl-Shift-Z</kbd> | redo |
| <kbd>Esc</kbd> while typing text | the color dialog |

## The key sheet

<kbd>?</kbd> (or <kbd>Alt-H</kbd>, <kbd>F12</kbd>) shows the live key sheet: the keys as they are bound right now, including your own overrides. The [command palette](/docs/getting-started/) shows each command's key too.

![The key sheet](/shots/key-sheet.png "The live key sheet, with your own bindings.")

Single-letter keys only work while a drawing tool is active. While you type text they type text instead.

## Your own keys

Override any key in the config file. Each action takes a list of key specs such as `"ctrl-s"`, `"alt-b"`, `"shift-f1"` or `"ctrl-shift-h"`:

```toml
[keymap]
preset = "modern"            # or "acid"

[keymap.bindings]
save = ["ctrl-s", "f2"]
```

`acidtrip keys` lists every action id, its title and its keys, grouped by category.

## The config file

The first run writes a config file with comments. Palette › *Open settings file* opens it, and *Reload settings* picks up your changes. Unknown keys are ignored, and anything you delete falls back to its default.

```toml
autosave_seconds = 20        # crash-recovery autosaves (0 = off)
version_every_minutes = 5    # version snapshots (0 = only on save)
backup = "bak"               # "none", "bak" or "numbered"

[new_doc]
kind = "classic"             # or "modern"
width = 80
height = 25
ice = true

[ai]
api_key = ""                 # empty: use ANTHROPIC_API_KEY
model = "claude-sonnet-5"
max_tool_rounds = 24

[share]
paste_url = "https://0x0.st"
png_scale = 2

[ui]
sidebar = true
show_grid = false
preview = true               # pixel-exact preview on kitty, sixel, iTerm
author = ""                  # SAUCE defaults for new documents
group = ""
name = ""                    # your name when drawing together
```

## Where things live

`acidtrip paths` prints where the config, fonts, stencils, versions, recovery files and sockets live. Set `ACIDTRIP_HOME` to keep everything in one directory.
