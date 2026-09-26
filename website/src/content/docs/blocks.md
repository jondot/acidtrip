---
title: Blocks and selection
nav: Blocks
description: Select an area, then copy, move, flip, justify, crop or reuse it, ACiDDraw style.
order: 10
glyph: "⬚"
---

## Selecting

<kbd>V</kbd>, or ⬚ in the tools grid, picks the select tool. Drag to select. <kbd>Ctrl-A</kbd> selects everything and <kbd>Esc</kbd> deselects. <kbd>Delete</kbd> or <kbd>Backspace</kbd> erases what's selected.

## The block menu

With a selection, <kbd>Enter</kbd> opens the ACiDDraw-style block menu (<kbd>Alt-B</kbd> in the `acid` keymap, or palette › *Block menu…*). Each item has its own key:

![The block menu open over a selection](/shots/block-menu.png "The block menu: one key per action.")

| Key | Action |
|---|---|
| <kbd>C</kbd> | Copy |
| <kbd>X</kbd> | Cut |
| <kbd>M</kbd> | Move (cut and carry) |
| <kbd>E</kbd> | Erase selection |
| <kbd>F</kbd> | Fill selection with brush |
| <kbd>O</kbd> | Outline selection with box |
| <kbd>H</kbd> / <kbd>V</kbd> | Flip horizontally / vertically, mirroring the glyphs too |
| <kbd>R</kbd> | Rotate 180° |
| <kbd>L</kbd> <kbd>N</kbd> <kbd>G</kbd> | Justify left, center, right |
| <kbd>D</kbd> | Delete block (shift left) |
| <kbd>K</kbd> | Crop |
| <kbd>S</kbd> | Save as a [stencil](/docs/fonts-stencils/) |
| <kbd>P</kbd> | Use the selection as a [pattern](/docs/patterns/) |
| <kbd>A</kbd> | Copy as ANSI |
| <kbd>I</kbd> | Ask AI about this block… |

## Copy and paste

<kbd>Ctrl-C</kbd>, <kbd>Ctrl-X</kbd> and <kbd>Ctrl-V</kbd> copy, cut and paste. Pasted blocks follow the mouse and stamp on every click.

Palette › *Copy selection as ANSI text* puts the selection on the clipboard as ANSI, for terminals and code blocks.

## Lines and columns

| Key | What it does |
|---|---|
| <kbd>Alt-I</kbd> / <kbd>Alt-Y</kbd> | insert / delete a line |
| <kbd>Alt-Shift-I</kbd> / <kbd>Alt-Shift-Y</kbd> | insert / delete a column |

## Undo

<kbd>Ctrl-Z</kbd> undoes and <kbd>Ctrl-Y</kbd> (or <kbd>Ctrl-Shift-Z</kbd>) redoes, with unlimited history. Every block action is one step.
