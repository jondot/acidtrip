---
title: Sharing
nav: Sharing
description: Copy your art as an image or text, publish a gist, or upload a PNG, from one menu.
order: 15
glyph: "↗"
---

## The share menu

<kbd>Alt-E</kbd> (or <kbd>Ctrl-Shift-E</kbd>, or palette › *Share…*) opens the share menu.

![The share menu with six options](/shots/share-menu.png "Alt-E: six ways to get the art out.")

1. **Copy as PNG image**, ready to paste into chat.
2. **Copy as ANSI (UTF-8)**, for terminals and code blocks.
3. **Copy as plain text.**
4. **Publish a GitHub gist**, as `.ans` and `.utf8ans`. This needs the `gh` command.
5. **Upload the PNG to a paste host** and copy the link.
6. **Export to file…**, in any [format](/docs/formats/).

## Paste host and image size

The paste host and the PNG scale are set in the `[share]` section of the config:

```toml
[share]
paste_url = "https://0x0.st"
png_scale = 2
```

## Other ways out

- Palette › *Copy selection as ANSI text* copies just the selection.
- The [Export panel](/docs/export/) writes many files at once.
- [Draw together](/docs/together/) shares the drawing live.
