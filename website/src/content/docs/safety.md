---
title: Never lose work
nav: Safety
description: Unlimited undo, crash recovery, automatic versions and backups.
order: 22
glyph: "◷"
---

## Undo

Every edit is undoable, with unlimited history. A brush stroke is one step, and an AI run is one step. <kbd>Ctrl-Z</kbd> undoes and <kbd>Ctrl-Y</kbd> or <kbd>Ctrl-Shift-Z</kbd> redoes.

## Recovery

Unsaved documents are autosaved every 20 seconds, with their edit history, so a restored piece still [replays](/docs/replay/). After a crash, the next start offers to restore them: <kbd>Enter</kbd> restores, <kbd>D</kbd> discards them all, and <kbd>Esc</kbd> asks again later.

## Versions

A version is saved on every save and every 5 minutes of editing. <kbd>Alt-V</kbd> (palette › *Version history…*) browses them with a live preview. <kbd>Enter</kbd> restores one, and <kbd>O</kbd> opens it as a copy in a new tab. Palette › *Save a named version…* saves one with a name you choose.

![Version history with a live preview](/shots/versions.png "Alt-V: every version, with a live preview.")

From the command line:

```sh
acidtrip versions art.acid                              # list them
acidtrip versions art.acid --restore 3f2a --out old.acid
```

## Backups

Saving over a file keeps a `.bak` copy, or numbered `.001`–`.999` copies like ACiDDraw.

## Settings

All of it is set in the config file:

```toml
autosave_seconds = 20       # 0 turns recovery autosave off
version_every_minutes = 5   # 0: versions only on save
backup = "bak"              # "none", "bak" or "numbered"
```

`acidtrip paths` shows where recovery files and versions live. See [Keymaps and config](/docs/keymaps-config/).
