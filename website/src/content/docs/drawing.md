---
title: Drawing and tools
nav: Drawing
description: Paint with the mouse or the keyboard, and every tool with its key.
order: 2
glyph: "▓"
---

## With the mouse

Drag on the canvas to paint. **<kbd>E</kbd> toggles the eraser**: press it, erase, press it again to go back. Picking a tile also returns you to the brush. Right-drag, Option-drag or Ctrl-drag also erase, in terminals that pass them through. iTerm2 may keep right-click for its own menu.

Click a tool in the sidebar's TOOLS grid, or press its key. Hover over a tool to see its name and key in the status bar. The tool's own panel sits right under the grid, with every option as a chip.

![acidtrip with shapes drawn on the canvas](/shots/shapes.png "Lines, rectangles and ellipses, drawn with a solid block.")

## From the keyboard

It works like ACiDDraw. The arrows move the cursor, and the number row types tiles from the active set, advancing like typing. The set is shown top-right as `1░ 2▒ 3▓ 4█ …`. Shift+arrows draw a trail with the current tile. <kbd>Space</kbd> applies the current tool, and for shapes a second <kbd>Space</kbd> finishes the shape.

<kbd>Home</kbd> and <kbd>End</kbd> go to the start and end of the line, <kbd>Ctrl-Home</kbd> and <kbd>Ctrl-End</kbd> to the first and last character on it, and <kbd>PageUp</kbd> / <kbd>PageDown</kbd> move a page.

## Every tool

Single-letter keys work while a drawing tool is active. While you type with the text tool they type text instead.

| Key | Tool | | Key | Tool |
|---|---|---|---|---|
| <kbd>B</kbd> | brush | | <kbd>G</kbd> | fill bucket |
| <kbd>T</kbd> | type text | | <kbd>I</kbd> | eyedropper |
| <kbd>P</kbd> | smart pen with brushes | | <kbd>F</kbd> | TheDraw / FIGlet text |
| <kbd>H</kbd> | half-block pixels | | <kbd>D</kbd> | gradient fill |
| <kbd>W</kbd> | pattern brush | | <kbd>N</kbd> | stencils |
| <kbd>L</kbd> <kbd>R</kbd> <kbd>O</kbd> | line, rectangle, ellipse | | <kbd>V</kbd> | select |
| <kbd>S</kbd> <kbd>C</kbd> <kbd>E</kbd> | shade, colorize, erase | | <kbd>M</kbd> | cycle mirror mode |
| <kbd>Shift-F</kbd> | photo filters | | <kbd>Shift-C</kbd> | recolor |
| <kbd>Ctrl-G</kbd> | the Art tool | | <kbd>Z</kbd> | zoom (pixel view) |
| <kbd>Tab</kbd> | cycle the tool's option | | <kbd>Shift-Tab</kbd> | shape look █┌╔╒╓╭ |

## What each tool does

- **Brush** (<kbd>B</kbd>) paints the glyph cell by cell. *paint* chooses what it lays down: *all* (glyph and colors), *color*, *fg* or *bg* only.
- **Text** (<kbd>T</kbd>): click, then type. *overwrite* or *insert*, and <kbd>Tab</kbd> jumps to the next tab stop. <kbd>Esc</kbd> returns to drawing.
- **Smart pen** (<kbd>P</kbd>) draws smooth strokes and picks the best glyph for each cell, with a library of brushes. See [Brushes and pen](/docs/brushes/).
- **Half-block pixels** (<kbd>H</kbd>) paint two pixels per cell with ▀ and ▄, so each cell can hold two colors. *pen* paints, *fill* floods.
- **Line, rectangle, ellipse** (<kbd>L</kbd> <kbd>R</kbd> <kbd>O</kbd>): drag the shape. Rectangles and ellipses are *outline* or *filled*, and *look* picks how they are drawn.
- **Fill bucket** (<kbd>G</kbd>) floods an area. *match* decides what counts as the same area: *all*, *char*, *color* or *bg*.
- **Gradient** (<kbd>D</kbd>) drags a color ramp over an area. See [Gradients](/docs/gradients/).
- **Eyedropper** (<kbd>I</kbd>): click to pick up a cell's glyph and colors. <kbd>Alt-U</kbd> picks up what's under the cursor from any tool.
- **Font** (<kbd>F</kbd>) stamps big text in TheDraw and FIGlet fonts. See [Fonts and stencils](/docs/fonts-stencils/).
- **Stencils** (<kbd>N</kbd>) stamp a saved piece of art: *transparent*, *opaque* or *under*.
- **Pattern brush** (<kbd>W</kbd>) paints with a repeating tile. See [Patterns](/docs/patterns/).
- **Shade** (<kbd>S</kbd>) steps cells through ░▒▓█: *denser* or *lighter* on each click.
- **Colorize** (<kbd>C</kbd>) recolors the ink and keeps the glyphs.
- **Erase** (<kbd>E</kbd>) erases to transparent.
- **Filters** (<kbd>Shift-F</kbd>) and **Recolor** (<kbd>Shift-C</kbd>) change the colors of what's there. See [Filters](/docs/filters/) and [Recolor](/docs/recolor/).
- **Select** (<kbd>V</kbd>): drag to select, then move or change it. See [Blocks and selection](/docs/blocks/).

The brush, pen, shapes, shade, colorize and erase tools share a *mirror* row: *off*, ↔, ↕ or ✚ for both. <kbd>M</kbd> cycles it. <kbd>-</kbd> and <kbd>=</kbd> make the brush smaller and bigger.

## Shapes follow the tile

The tile you picked decides how shapes are drawn. A solid block draws lines, rectangles and ellipses at half-block resolution, so circles come out smooth. A box-drawing character draws the matching frame. Anything else is drawn with that glyph. <kbd>Shift-Tab</kbd> or the *look* chips pick the look: █ pixels, ┌ ╔ ╒ ╓ frames (plus ╭ rounded in Modern documents), or the brush's own glyph.

## View

<kbd>Z</kbd> toggles zoom, a pixel view where half-block drawing is exact. <kbd>Alt-G</kbd> toggles grid guides, <kbd>Alt-W</kbd> the pixel-exact preview, and <kbd>Alt-Shift-W</kbd> plays the piece at modem speed.
