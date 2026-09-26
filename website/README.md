# acidtrip website

The project site, built as the acidtrip editor itself. The pages are open files in the tab bar and layers in the LAYERS panel. The COLORS panel sets the accent and the canvas, CHARACTERS sets the glyph used for the rules, Ctrl-K opens a palette that searches every page and doc heading, and the headings are lettered in TheDraw fonts by acidtrip. It is a static site built with [Astro](https://astro.build), with no framework on the client: the scripts in `src/scripts/` are plain TypeScript.

## Develop

```sh
npm install
npm run dev        # http://localhost:4321
```

## Build

```sh
npm run build      # static site in dist/
npm run preview    # serve dist/
```

To host it under a path, for example on GitHub Pages, set the base:

```sh
SITE_BASE=/acidtrip/ npm run build
```

## Layout

| Path | What |
|---|---|
| `src/layouts/Canvas.astro` | the editor frame every page lives in |
| `src/components/` | tab bar, sidebar panels, status bar, command palette, lightbox, `Shot`, `Heading`, `Term` |
| `src/pages/` | home, features, gallery, download, docs index, one page per doc |
| `src/content/docs/*.md` | the docs; `order` in the front matter sets their order |
| `src/lib/site.ts` | the GitHub link, the install line, the page list |
| `src/lib/rehype-acid.mjs` | Markdown to the editor's look: screenshots, tables, terminal blocks |
| `src/styles/editor.css`, `src/scripts/editor.ts` | the design and its behaviour |
| `public/shots/` | screenshots of the real app (generated, committed) |
| `public/art/` | the lettered headings (generated, committed) |

In a doc, an image with a title on a line of its own becomes a framed screenshot with a caption. If the PNG doesn't exist, the image is dropped from the page:

```md
![The export panel](/shots/export-panel.png "The EXPORT panel. One row per file.")
```

Keys and commands in the docs are checked against `crates/acidtrip/src/keymap.rs`, `actions.rs` and `cli.rs`. Keep them that way when either side changes.

## Screenshots

Each screenshot is taken from the real app by the test harness (`acidtrip-harness`), at 120×40:

```sh
scripts/shots.sh              # every shots/*.at
scripts/shots.sh gradient     # just shots/gradient.at
```

A scenario's `shot NAME` becomes `public/shots/NAME.png`. The scenarios run in a scratch directory that holds a copy of `shots/art/` and the repo's test corpus, with a fresh `ACIDTRIP_HOME`, so nothing the app writes lands in the repo. TheDraw fonts are linked from your own library; get them once with `acidtrip fonts get`. The build uses `CARGO_TARGET_DIR` if it is set.

The art the screenshots show is drawn by acidtrip's own MCP tools and is seeded, so it comes out the same every time:

```sh
python3 scripts/make-art.py   # writes shots/art/sunset.acid and .ans
```

## Headings

The lettered headings are set by acidtrip's `banner` tool over MCP, then cropped to whole cells:

```sh
python3 scripts/headings.py   # writes public/art/*.png
```

It needs Pillow and the TheDraw fonts. Add a doc and rerun it to get that doc's `doc-<slug>.png` heading.

## Deploy

`npm run build` writes a plain static site to `dist/`; any static host serves it. The live site is https://acidtrip.vercel.app.
