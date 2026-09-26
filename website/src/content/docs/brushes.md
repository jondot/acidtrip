---
title: Brushes and the smart pen
nav: Brushes
description: A pen that picks the best glyph for every cell, and a library of brushes that make it a paint program.
order: 5
glyph: "✎"
---

## The smart pen

<kbd>P</kbd>, or ✎ in the tools grid, picks the smart pen. It draws your stroke at the glyph's own 8×16-pixel resolution. Then, for each cell, it picks the glyph from the active set whose real bitmap best matches what the stroke covered: `▀ ▄ ▌ ▐` for clean edges, `░ ▒ ▓` to smooth slopes, and connected `─ │ ┌ ┘` with a box-drawing set.

With terminals that report exact mouse pixels (kitty, WezTerm, Ghostty, xterm and others) it follows the pointer inside the cell.

![A stroke drawn with the smart pen](/shots/pen.png "The smart pen picks a glyph per cell from what the stroke covered.")

## The brush library

Brushes make the pen a paint program. <kbd>Tab</kbd> cycles the presets, and <kbd>-</kbd> and <kbd>=</kbd> resize.

| Brush | What it does |
|---|---|
| Ink | the crisp smart pen |
| Brush pen | tapers at both ends, thins when you move fast |
| Marker | bold square tip, blocks |
| Calligraphy | 45° flat nib: thick one way, thin the other |
| Airbrush | soft edge, builds up `░ ▒ ▓ █` where you go over it |
| Soft shade | 60% tone that never goes solid, for shading |
| Chalk | paper grain breaks the ink up |
| Spray | scattered dots `· ∙ • °` |
| ASCII | text-art characters `_ ^ * / \ o` |
| Line art | connected single box lines |

The pen has the same *mirror* row as the brush: *off*, ↔, ↕ or ✚. <kbd>M</kbd> cycles it.

## The brush studio

<kbd>Shift-P</kbd>, the ⚙ in the pen's sidebar panel, or palette › *Brush studio…* opens the studio, with sliders and a live preview stroke.

![The brush studio with sliders and a preview stroke](/shots/brush-studio.png "The brush studio: every setting, with a stroke that updates as you go.")

Each brush is made of four parts:

- **Tip:** size, hardness, roundness, angle, square.
- **Ink:** opacity, flow, spacing, scatter, count, grain.
- **The hand:** taper, speed thinning, and a streamline stabilizer.
- **Glyphs:** the tile set, blocks, shades, ASCII, dots or lines.

In the studio, <kbd>Tab</kbd> switches pane, <kbd>↑</kbd> <kbd>↓</kbd> pick a setting and <kbd>←</kbd> <kbd>→</kbd> adjust it. <kbd>R</kbd> resets, and <kbd>Enter</kbd> or <kbd>Esc</kbd> closes.

## Your own presets

<kbd>S</kbd> in the studio saves your tweaks as a preset in the library's `brushes/` folder, one small TOML file each. A preset saved under a built-in's name replaces it. <kbd>D</kbd> deletes a preset.

The AI paints with the same brushes, through its `brush_stroke` tool. See [AI](/docs/ai/).
