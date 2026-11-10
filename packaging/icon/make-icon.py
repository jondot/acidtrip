#!/usr/bin/env python3
"""Regenerate the app icons from the site's 16x16 pixel mark.

    python3 packaging/icon/make-icon.py   (needs Pillow)

Writes acidtrip-<size>.png and acidtrip.icns next to this script.
"""
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).parent
MAGENTA, CYAN = (255, 85, 255, 255), (85, 255, 255, 255)
# The favicon's rectangles (website/public/favicon.svg), in 16x16 units.
MARK = [
    ((2, 2, 6, 12), MAGENTA),
    ((8, 2, 6, 6), CYAN),
    ((8, 10, 2, 2), CYAN),
    ((12, 10, 2, 2), CYAN),
    ((10, 12, 2, 2), CYAN),
]


def icon(size=1024):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # macOS grid: an 824/1024 rounded square, centered.
    inset = size * 100 // 1024
    d.rounded_rectangle(
        (inset, inset, size - inset, size - inset), radius=size * 185 // 1024, fill=(12, 12, 16, 255)
    )
    unit = size * 36 // 1024
    off = (size - unit * 12) // 2 - 2 * unit  # the mark spans x/y 2..14
    for (x, y, w, h), c in MARK:
        d.rectangle((off + x * unit, off + y * unit, off + (x + w) * unit - 1, off + (y + h) * unit - 1), fill=c)
    return img


if __name__ == "__main__":
    big = icon()
    for s in (16, 32, 48, 64, 128, 256, 512, 1024):
        (big if s == 1024 else big.resize((s, s), Image.LANCZOS)).save(HERE / f"acidtrip-{s}.png")
    big.save(HERE / "acidtrip.icns", sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
