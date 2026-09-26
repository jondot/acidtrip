---
title: The Art tool
nav: Art tool
description: Turn the keyboard into a glyph board, so the left hand types blocks and the right hand moves.
order: 4
glyph: "▚"
---

## The glyph board

<kbd>Ctrl-G</kbd>, or ▚ in the tools grid, starts the Art tool. The keyboard becomes a board of glyphs: the left hand types blocks and the right hand moves. It has its own panel in the sidebar, and picking any other tool leaves it. <kbd>Esc</kbd> or <kbd>Ctrl-G</kbd> leaves too.

![The Art tool panel showing the keyboard with a glyph on each key](/shots/art-tool.png "The Art tool: each key shows the glyph it types, in your colors.")

## Spatial keys

<kbd>Q</kbd> <kbd>W</kbd> <kbd>E</kbd> / <kbd>A</kbd> <kbd>S</kbd> <kbd>D</kbd> / <kbd>Z</kbd> <kbd>X</kbd> <kbd>C</kbd> are the nine parts of a cell, each key's glyph sitting where the key sits:

| | | |
|---|---|---|
| <kbd>Q</kbd> | <kbd>W</kbd> ▀ | <kbd>E</kbd> |
| <kbd>A</kbd> ▌ | <kbd>S</kbd> █ | <kbd>D</kbd> ▐ |
| <kbd>Z</kbd> | <kbd>X</kbd> ▄ | <kbd>C</kbd> |

The corner keys hold quadrants in Modern documents. Classic documents keep to CP437, so the corners type CP437 textures instead. <kbd>T</kbd> <kbd>G</kbd> <kbd>B</kbd> and <kbd>F</kbd> <kbd>V</kbd> hold the details that go with the set.

<kbd>1</kbd>–<kbd>5</kbd> are always the shades ░▒▓█■.

## Glyph sets

<kbd>[</kbd> <kbd>]</kbd>, or ‹ › on the panel, rotate the board through the sets: Blocks, Single lines, Double lines, the two Mixed line sets (╓ and ╒), and, in Modern documents, Quads and Eighths.

The sets come from counting glyphs across 500+ scene files from 1994 to 2023: the glyphs artists actually reach for, placed where your fingers expect them.

## The right hand

| Key | What it does |
|---|---|
| <kbd>I</kbd> <kbd>J</kbd> <kbd>K</kbd> <kbd>L</kbd> | move; hold <kbd>Shift</kbd> to draw a trail |
| <kbd>Y</kbd> / <kbd>H</kbd> | undo / redo |
| <kbd>U</kbd> / <kbd>O</kbd> | cycle the foreground |
| <kbd>M</kbd> / <kbd>,</kbd> | cycle the background |
| <kbd>R</kbd> or <kbd>Space</kbd> | erase the cell |

Typing a glyph onto the same glyph erases it too.

## Your own board

The panel shows the keyboard with each key's glyph in your colors. Click a key to give it another glyph; that's saved per set. The mouse paints with the last glyph you typed.
