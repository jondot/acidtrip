#!/usr/bin/env python3
"""Draw the art the screenshots show, with acidtrip's own MCP tools.

Writes shots/art/sunset.acid (and .ans): a synthwave sunset over a city,
lettered with a TheDraw font, on four named layers. Seeded, so it comes out
the same every time. Needs the TheDraw fonts (`acidtrip fonts get`, once).

    python3 scripts/make-art.py [path/to/acidtrip]
"""
import os
import random

from acidmcp import SITE, Mcp

OUT = os.path.join(SITE, 'shots', 'art')
TOP = 8  # rows above the picture, for the lettering


def main():
    os.makedirs(OUT, exist_ok=True)
    random.seed(7)
    m = Mcp()
    m.tool('new_canvas', width=80, height=TOP + 24, kind='classic', ice=True)
    py = TOP * 2  # pixel row where the picture starts

    m.tool('layer', action='set_props', index=0, name='sky')
    # Sky: dithered bands from night blue to a red horizon.
    bands = [(0, 4, 0, 1, .0), (4, 4, 0, 1, .2), (8, 4, 1, 0, .3), (12, 4, 1, 5, .35), (16, 4, 5, 1, .3),
             (20, 4, 5, 4, .4), (24, 4, 4, 5, .35), (28, 4, 4, 12, .45)]
    for y, h, c, c2, mix in bands:
        m.tool('pixel_rect', x=0, y=py + y, w=80, h=h, color=c, color2=c2, mix=mix)
    stars = [[random.randrange(80), py + random.randrange(0, 14), random.choice([15, 7, 8, 7])] for _ in range(26)]
    m.tool('pixel_set', pixels=stars)
    # Sea, with the sun's reflection.
    m.tool('pixel_rect', x=0, y=py + 32, w=80, h=16, color=0, color2=1, mix=.45)
    for y, w, c in [(33, 22, 14), (35, 18, 12), (37, 14, 14), (39, 10, 12), (41, 8, 4), (43, 5, 4), (45, 3, 4)]:
        m.tool('pixel_rect', x=40 - w // 2, y=py + y, w=w, h=1, color=c)

    m.tool('layer', action='add', name='sun')
    m.tool('pixel_ellipse', cx=40, cy=py + 26, rx=13, ry=13, color=12)
    m.tool('pixel_ellipse', cx=40, cy=py + 24, rx=11, ry=11, color=14)
    for y in (22, 25, 27, 29, 31):
        m.tool('pixel_rect', x=26, y=py + y, w=29, h=1, color=0)
    # The sea covers the sun's lower half.
    m.tool('erase_rect', x=0, y=TOP + 16, w=80, h=8)

    m.tool('layer', action='add', name='city')
    x, blocks = 0, []
    while x < 80:
        w = random.randint(3, 7)
        h = random.randint(2, 6) if 30 < x < 50 else random.randint(4, 13)
        blocks.append((x, w, h))
        x += w
    for x, w, h in blocks:
        m.tool('pixel_rect', x=x, y=py + 32 - h, w=w, h=h, color=0)
    windows = []
    for x, w, h in blocks:
        for yy in range(32 - h + 1, 31, 2):
            for xx in range(x + 1, x + w - 1, 2):
                if random.random() < .3:
                    windows.append([xx, py + yy, random.choice([14, 6, 6, 11])])
    m.tool('pixel_set', pixels=windows)

    m.tool('layer', action='add', name='title')
    m.tool('banner', font='amnesiax#0', text='acidtrip', x=0, y=0, center=True)
    m.tool('put_text', x=71, y=TOP + 23, text='acidtrip', fg=8, bg=0, transparent_spaces=True)

    for ext in ('acid', 'ans'):
        print(m.tool('save', path=os.path.join(OUT, 'sunset.' + ext), title='Sunset', author='acidtrip',
                     group='acidtrip'))
    m.tool('render_png', path=os.path.join(OUT, 'sunset.png'), scale=1)


if __name__ == '__main__':
    main()
