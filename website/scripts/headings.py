#!/usr/bin/env python3
"""Letter the site's headings with acidtrip itself.

Drives `acidtrip mcp --headless` over stdio: a blank canvas, the `banner`
tool with a TheDraw font, then `render_png`. The PNGs are cropped to the
art (whole 8x16 cells) and written to public/art/. Needs Pillow and the
TheDraw fonts (`acidtrip fonts get`, once).

    python3 scripts/headings.py [path/to/acidtrip]
"""
import os
import re
import sys
import tempfile

from PIL import Image

from acidmcp import SITE, Mcp

OUT = os.path.join(SITE, 'public', 'art')


def letter(m, name, font, text):
    m.tool('new_canvas', width=220, height=14, kind='classic', ice=True)
    m.tool('banner', font=font, text=text, x=0, y=0)
    with tempfile.TemporaryDirectory() as d:
        tmp = os.path.join(d, 'h.png')
        m.tool('render_png', path=tmp, scale=1)
        im = Image.open(tmp).convert('RGB')
    box = im.getbbox()
    if not box:
        sys.exit(f'{name}: nothing drawn (is the font "{font}" installed? run `acidtrip fonts get`)')
    w = (box[2] + 7) // 8 * 8
    h = (box[3] + 15) // 16 * 16
    im.crop((0, 0, w, h)).save(os.path.join(OUT, name + '.png'), optimize=True)
    print(f'{name}.png {w}x{h}')


def doc_titles():
    """(slug, nav) for every doc, from the frontmatter."""
    d = os.path.join(SITE, 'src', 'content', 'docs')
    for f in sorted(os.listdir(d)):
        if f.endswith(('.md', '.mdx')):
            text = open(os.path.join(d, f)).read()
            nav = re.search(r'^nav:\s*"?([^"\n]+)"?\s*$', text, re.M)
            yield f.rsplit('.', 1)[0], nav.group(1).strip() if nav else f


def main():
    os.makedirs(OUT, exist_ok=True)
    m = Mcp()
    # The front page logo, and a narrow one for phones.
    letter(m, 'logo-acidtron', 'acidtron', 'ACiDTRiP')
    letter(m, 'logo-amnesia', 'amnesiax#0', 'acidtrip')
    letter(m, 'logo-acid3d', 'acid3dx#1', 'ACiDTRiP')
    letter(m, 'logo-1911', '1911x#1', 'ACiDTRiP')
    # Front page feature headings.
    for w in ['draw', 'together', 'replay', 'gallery', 'studio', 'formats', 'export', 'frames']:
        letter(m, 'h-' + w, 'amnesiax#0', w)
    # Page headings.
    for slug, text in [('features', 'TOOLS'), ('gallery', 'GALLERY'), ('download', 'DOWNLOAD'), ('docs', 'DOCS')]:
        letter(m, 'page-' + slug, 'acidx#3', text)
    for slug, nav in doc_titles():
        letter(m, 'doc-' + slug, 'acidx#3', nav.upper())


if __name__ == '__main__':
    main()
