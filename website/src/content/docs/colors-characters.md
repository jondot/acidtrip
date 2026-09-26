---
title: Colors and characters
nav: Colors
description: The FG and BG boxes, the palette, iCE colors, and the fifteen character sets.
order: 3
glyph: "■"
---

## Foreground and background

Colors work like Paint. The sidebar's COLORS panel has an **FG** box and a **BG** box. Click a box to make it the active one, then click a palette color to set it. Everything works with one mouse button. Right-click sets the other box, where the terminal passes it through.

| Key | What it does |
|---|---|
| <kbd>X</kbd> | swap FG and BG |
| <kbd>,</kbd> <kbd>.</kbd> | previous / next foreground color |
| <kbd>;</kbd> <kbd>'</kbd> | previous / next background color |
| <kbd>Ctrl-↑</kbd> <kbd>Ctrl-↓</kbd> | previous / next foreground, from anywhere |
| <kbd>Ctrl-←</kbd> <kbd>Ctrl-→</kbd> | previous / next background, from anywhere |
| <kbd>Alt-C</kbd> | the full color dialog |
| <kbd>Alt-U</kbd> | pick up the colors under the cursor |

Clicking the box that is already active opens the full color dialog too. In it, ↑ ↓ change the foreground and ← → the background, and Modern documents can type any RGB color as hex.

![The color dialog](/shots/colors-dialog.png "The full color dialog.")

## iCE colors

Classic ANSI has 16 foreground colors but only 8 backgrounds; the bright half of the background colors meant blinking text. **iCE colors** use them as bright backgrounds instead. <kbd>Alt-Z</kbd> toggles iCE, and so does clicking `iCE` in the status bar. Without iCE, backgrounds stay in the dark 8.

New documents have iCE on. The `ice` key in the `[new_doc]` section of the config changes that.

## Character sets

The characters bar is ACiDDraw's: ten glyphs at a time, from 15 sets taken from ACiDDRAW.EXE itself. They cover single and double lines, the two mixed line sets, crossings, blocks and shades, symbols, arrows, math, Greek and accented Latin letters. Blocks and shades is the set you start with.

- <kbd>1</kbd>–<kbd>0</kbd> place the active set's glyph at the cursor and move right. With the mouse, a key puts the glyph where the pointer is.
- <kbd>[</kbd> <kbd>]</kbd> step through the sets. <kbd>Alt-P</kbd> and <kbd>Alt-N</kbd> do the same from anywhere.
- <kbd>Alt-1</kbd>–<kbd>Alt-0</kbd> (or <kbd>F1</kbd>–<kbd>F10</kbd>) place a glyph even while you type text.
- The set is shown top-right as `1░ 2▒ 3▓ 4█ …`, and in the CHARACTERS panel with ‹ › to change it.

Clicking a glyph in the sidebar makes it the brush, and a ghost shows under the pointer before you paint.

## The character picker

<kbd>Alt-K</kbd> opens the full picker: the full CP437 grid in Classic documents, plus a browser of Unicode blocks in Modern ones.

![The character picker](/shots/char-picker.png "Alt-K: every character, not just the ten in the bar.")
