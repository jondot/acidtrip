<p align="center">
  <img src="website/public/art/logo-acid3d.png" alt="acidtrip" width="640">
</p>

<p align="center">
  <b>ANSI art in your terminal, the way ACiDDraw did it, rebuilt for today.</b><br>
  Mouse-first. Every classic scene format. TheDraw fonts. Layers, frames, replay.<br>
  Draw with a friend over the internet, or with Claude.
</p>

<p align="center">
  <a href="https://github.com/jondot/acidtrip/releases/latest"><img src="https://img.shields.io/github/v/release/jondot/acidtrip?color=00a8a8&label=release" alt="release"></a>
  <a href="https://github.com/jondot/acidtrip/actions/workflows/ci.yml"><img src="https://github.com/jondot/acidtrip/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-a800a8" alt="license">
  <a href="https://acidtrip.vercel.app"><img src="https://img.shields.io/badge/docs-acidtrip.vercel.app-fcfc54" alt="docs"></a>
</p>

<p align="center">
  <img src="website/public/shots/overview.png" alt="acidtrip editing a synthwave sunset" width="900">
</p>

```sh
curl -fsSL https://raw.githubusercontent.com/jondot/acidtrip/main/install.sh | sh
acidtrip              # a new canvas
acidtrip logo.ans     # open (or create) a file
```

macOS and Linux, x86_64 and arm64. Any terminal with truecolor and a mouse works: iTerm2, kitty, WezTerm, Ghostty, Alacritty. To build from source instead, run `cargo install --git https://github.com/jondot/acidtrip acidtrip`.

<br>

<p align="center"><img src="website/public/art/h-draw.png" alt="draw" height="56"></p>

Paint with the mouse and watch the art come out in real CP437 glyphs. The **smart pen** traces your stroke at the font's own 8×16 pixels and picks the block, shade or box line that fits each cell. **Brushes** turn it into a paint program: brush pen, calligraphy, airbrush, chalk and spray, each tunable in the brush studio.

<table>
  <tr>
    <td width="50%"><img src="website/public/shots/pen.png" alt="smart pen strokes"><br><sub>Smooth strokes, picked glyph by glyph</sub></td>
    <td width="50%"><img src="website/public/shots/brush-studio.png" alt="brush studio"><br><sub>The brush studio: tip, ink, hand, glyphs</sub></td>
  </tr>
  <tr>
    <td><img src="website/public/shots/gradient-panel.png" alt="gradient fill"><br><sub>Gradients shaded ░▒▓ the way scene artists did it</sub></td>
    <td><img src="website/public/shots/pattern-browser.png" alt="pattern browser"><br><sub>Seamless patterns: bricks, weaves, grids, your own</sub></td>
  </tr>
  <tr>
    <td><img src="website/public/shots/filters.png" alt="photo filters"><br><sub>43 photo filters, from Vivid to Ludwig</sub></td>
    <td><img src="website/public/shots/recolor.png" alt="recolor"><br><sub>Recolor: swap one color everywhere</sub></td>
  </tr>
  <tr>
    <td><img src="website/public/shots/block-menu.png" alt="block menu"><br><sub>The ACiDDraw block menu: move, flip, rotate, justify</sub></td>
    <td><img src="website/public/shots/layers-panel.png" alt="layers"><br><sub>Layers, with lock and tracing references</sub></td>
  </tr>
</table>

Keys are what ACiDDraw players expect: the arrows move and `1`–`0` type the tile set. Or switch the keyboard to the **Art tool** and it becomes a glyph board, with `W` for ▀, `S` for █ and `X` for ▄. Everything is also a click away in the sidebar, and `Ctrl-K` searches every command.

About 3,700 **TheDraw fonts** are one command away (`acidtrip fonts get`), with the FIGlet set built in. Pull in any **image**, and every cell tries every block glyph to find the best shape and two colors.

<table>
  <tr>
    <td width="50%"><img src="website/public/shots/font-dialog.png" alt="TheDraw font picker"><br><sub>Pick a TheDraw font, see it live</sub></td>
    <td width="50%"><img src="website/public/shots/import-image.png" alt="image import"><br><sub>Image import with presets: photo, scene, pixel art, cel, comic</sub></td>
  </tr>
</table>

<p align="center"><img src="website/public/art/h-frames.png" alt="frames" height="56"></p>

Pieces can have **frames**, with onion skin and timing, and export to an animated GIF or a classic `cat`-able ansimation. Every edit is recorded too, so **replay** plays a piece back being drawn, speed-paint style, and saves it as a GIF or an asciinema cast.

<p align="center"><img src="website/public/art/h-together.png" alt="together" height="56"></p>

<table>
  <tr>
    <td width="50%"><img src="website/public/shots/together-panel.png" alt="draw together"><br><sub><b>Draw together:</b> share a ticket, everyone draws on one canvas. Peer to peer, no accounts, no servers to run.</sub></td>
    <td width="50%"><img src="website/public/shots/ai-prompt.png" alt="AI prompt"><br><sub><b>Claude as a partner:</b> ask for a logo or a fire gradient, and it draws on its own layer with the real tools.</sub></td>
  </tr>
</table>

Claude Code can draw straight into your open editor, live and undoable, with no API key needed in acidtrip:

```sh
claude mcp add acidtrip -- acidtrip mcp
```

<p align="center"><img src="website/public/art/h-gallery.png" alt="gallery" height="56"></p>

Browse decades of scene art on **16colo.rs** like a streaming service, view any piece full screen with its credits, and open it as a fresh document to remix. The **sourcing studio** cuts letters from classic logos into fonts in the artist's style, with the artist credited.

<table>
  <tr>
    <td width="50%"><img src="website/public/shots/gallery-home.png" alt="gallery home"><br><sub>Rows of groups, packs and years</sub></td>
    <td width="50%"><img src="website/public/shots/studio-cutter.png" alt="sourcing studio"><br><sub>Cut letters from a logo into a font</sub></td>
  </tr>
</table>

<p align="center"><img src="website/public/art/h-export.png" alt="export" height="56"></p>

It loads and saves ANSI (iCE and 24-bit), BIN, XBin, ADF, IDF, TND, PCBoard, Avatar and ASCII. It exports to pixel-exact PNG and SVG, GIFs (including the BBS modem reveal), HTML, a React component, C/Pascal/ASM arrays, mIRC and asciinema. Set up the files once in the **export panel**, Figma style, and write them all with one click.

<table>
  <tr>
    <td width="50%"><img src="website/public/shots/export-panel.png" alt="export panel"><br><sub>Export rows: every size and format, one click</sub></td>
    <td width="50%"><img src="website/public/shots/versions.png" alt="versions"><br><sub>Versions, autosave and crash recovery</sub></td>
  </tr>
</table>

```sh
acidtrip convert art.ans art.svg --pixel-exact
acidtrip convert art.ans art.gif --gif reveal --baud 9600
acidtrip replay art.acid art.gif
```

<br>

## Made with acidtrip

A set of [Omarchy](https://omarchy.org) wallpapers and tee logos, drawn in acidtrip. The sources (`.acid`, `.ans`) and PNGs at desktop sizes are in [examples/](examples/).

<table>
  <tr>
    <td width="50%"><img src="examples/omarchy/wallpapers/omarchy-quattro.png" alt="Quattro"></td>
    <td width="50%"><img src="examples/omarchy/wallpapers/omarchy-outrun.png" alt="Outrun"></td>
  </tr>
  <tr>
    <td><img src="examples/omarchy/wallpapers/omarchy-neon-city.png" alt="Neon city"></td>
    <td><img src="examples/omarchy/wallpapers/omarchy-new-horizon.png" alt="New horizon"></td>
  </tr>
  <tr>
    <td><img src="examples/omarchy/wallpapers/omarchy-jade-peaks.png" alt="Jade peaks"></td>
    <td><img src="examples/omarchy/wallpapers/omarchy-industrial-moon.png" alt="Industrial moon"></td>
  </tr>
  <tr>
    <td align="center"><img src="examples/omarchy/tees/tee-omarchy-neon.png" alt="Omarchy neon tee" width="80%"></td>
    <td align="center"><img src="examples/omarchy/tees/tee-omarchy-drip.png" alt="Omarchy drip tee" width="80%"></td>
  </tr>
</table>

## Learn more

The full manual is at **[acidtrip.vercel.app](https://acidtrip.vercel.app/docs/)**. It covers [drawing](https://acidtrip.vercel.app/docs/drawing/), [brushes](https://acidtrip.vercel.app/docs/brushes/), [the Art tool](https://acidtrip.vercel.app/docs/art-tool/), [fonts and stencils](https://acidtrip.vercel.app/docs/fonts-stencils/), [animation](https://acidtrip.vercel.app/docs/animation/), [replay](https://acidtrip.vercel.app/docs/replay/), [drawing together](https://acidtrip.vercel.app/docs/together/), [AI](https://acidtrip.vercel.app/docs/ai/), [formats](https://acidtrip.vercel.app/docs/formats/), [the CLI](https://acidtrip.vercel.app/docs/cli/), and [keymaps and config](https://acidtrip.vercel.app/docs/keymaps-config/). Inside the app, `?` shows the live key sheet.

## Development

```sh
cargo test --workspace     # unit tests, plus end-to-end runs of the real app
```

`acidtrip-harness` drives the real binary in a pseudo-terminal with keys and the mouse, and screenshots it. Every screenshot above was taken that way. The scenarios in `tests/e2e/` and the user flows in `tests/flows/` run as part of `cargo test`.

| Crate | Contents |
|---|---|
| `acidtrip-core` | model, transactions, tools, rendering |
| `acidtrip-io` | formats, fonts, stencils, versions |
| `acidtrip-ai` | tool schema, MCP, agent, harvester |
| `acidtrip-net` | draw together: peer-to-peer sessions over iroh |
| `acidtrip-harness` | the headless pty test and screenshot harness |
| `acidtrip` | the TUI |

The website lives in `website/` (see [website/README.md](website/README.md)). Releases are one command: see [RELEASING.md](RELEASING.md).

## Credits

- ACiD Productions, for ACiDDraw.
- [ansidraw](https://github.com/jeffreygnatek/ansidraw) (MIT), whose MCP and half-block ideas shaped this project.
- [icy_tools](https://github.com/mkrueger/icy_tools) (MIT/Apache), for the ANSI parser, `retrofont` and `icy_sauce`.
- The VGA font is from [libansilove](https://www.ansilove.org) (BSD-2).
- The FIGlet fonts are BSD-3.
- GNU Unifont, for the Unicode fallback.

Licensed MIT OR Apache-2.0.
