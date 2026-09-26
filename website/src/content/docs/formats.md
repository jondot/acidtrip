---
title: Formats and importing
nav: Formats
description: Every format acidtrip opens and writes, and how pictures become art.
order: 14
glyph: "▣"
---

## Load and save

| | Load | Save |
|---|---|---|
| ANSI `.ans` (iCE, 24-bit PabloDraw), UTF-8 ANSI | ✓ | ✓ |
| BIN, XBin, Artworx ADF, iCE Draw IDF, TundraDraw TND | ✓ | ✓ |
| PCBoard, Avatar, ASCII | ✓ | ✓ |
| `.acid` (native: layers + metadata + edit history) | ✓ | ✓ |
| PNG (imported as art; pixel-exact VGA render on export) | ✓ | ✓ |
| GIF (still, BBS "modem reveal", layers as frames), SVG (text or pixel-exact) | | ✓ |
| HTML page, React `.tsx` component, C/Pascal/ASM arrays, mIRC, asciinema cast | | ✓ |

Classic documents save losslessly to `.ans`, `.xb` and `.bin`. `.acid` keeps everything: layers, frames, export rows, and the edit history that [Replay](/docs/replay/) plays.

## From the command line

```sh
acidtrip convert art.ans art.svg --pixel-exact
acidtrip convert art.ans art.gif --gif reveal --baud 9600
acidtrip render art.xb art.png --scale 2
```

See [Command line](/docs/cli/) for every flag.

## Importing images

Palette › *Import image…* turns a PNG, JPEG, GIF, WebP or BMP into art. Pick it from the list (images here, plus Downloads, Desktop and Pictures, newest first), type a path, or drop the file on the window.

![The import dialog with presets and a live preview](/shots/import-image.png "Import image: presets, settings and a live preview.")

## Presets

Presets set everything at once:

- **photo**
- **scene**: 16 colors, ░▒▓ dithering, CP437 only
- **pixel art**, kept sharp
- **cel**, for anime and flat-colored drawings: outlines stay dark, fills stay flat
- **comic**, for ink drawings and comics: bolder lines
- **line art**
- **ascii**

The dialog looks at the picture and picks one ("looks like cel"); <kbd>1</kbd>–<kbd>7</kbd> switch.

## Settings

Each setting is a `‹ value ›` row you click or change with <kbd>←</kbd> <kbd>→</kbd>, and the preview updates as you go.

- **Keep lines** finds thin dark strokes before the image is shrunk and carries them into the glyphs, so outlines survive at 80 columns instead of fading into the fill.
- **Photo filter** runs any of the [Filters](/docs/filters/) presets on the picture before it is converted.

Every cell tries each block glyph and picks the shape and two colors that match best. Modern documents also get quadrants, sextants and eighths in truecolor. Classic documents can use their own 16 colors, 16 fitted to the image (for a new document), or switch to truecolor.

The result goes into a new layer, a new document, or a floating stamp you place.
