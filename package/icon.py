#!/usr/bin/env python3
"""Draws the application icon.

The icon is the spectrum: four bars on a dark tile, three of them short and
teal, one long and amber crossing the vertical line that marks the required
accuracy. That picture is the whole application, and at sixteen pixels it
still reads as "one of these is not like the others".

Written rather than drawn so that it can be regenerated and argued with.
No dependencies: PNG is a zlib stream with a four-byte-per-pixel payload,
and everything here is axis-aligned rectangles with rounded corners, so a
four-times supersample is all the antialiasing it needs.

    python3 package/icon.py build/icon.iconset
    iconutil -c icns build/icon.iconset -o package/rigidity.icns
"""

import os
import struct
import sys
import zlib

SUPERSAMPLE = 4

# The palette's own colours, so the icon cannot drift from the application.
TILE = (0x14, 0x16, 0x19)
LINE = (0x6A, 0xA9, 0xFF)
GOOD = (0x4F, 0xB8, 0xA8)
SHORT_OF_IT = (0xE0, 0xB3, 0x41)

# Fractions of the tile. The bars start at a common left edge, which is what
# makes their ends comparable — the same reason the panel draws them so.
BARS = [(0.30, 0.30, GOOD), (0.44, 0.34, GOOD), (0.58, 0.44, GOOD),
        (0.72, 0.74, SHORT_OF_IT)]
BAR_LEFT = 0.16
BAR_HEIGHT = 0.075
TOLERANCE_AT = 0.66


def rounded(x, y, left, top, right, bottom, radius):
    """Whether a point is inside a rounded rectangle."""
    if not (left <= x <= right and top <= y <= bottom):
        return False
    for cx, cy in ((left + radius, top + radius), (right - radius, top + radius),
                   (left + radius, bottom - radius), (right - radius, bottom - radius)):
        if (x < left + radius or x > right - radius) and (y < top + radius or y > bottom - radius):
            if abs(x - cx) <= radius and abs(y - cy) <= radius:
                return (x - cx) ** 2 + (y - cy) ** 2 <= radius * radius
    return True


def draw(size):
    """One icon, as RGBA rows."""
    n = size * SUPERSAMPLE
    # macOS leaves the outer eighth of the grid empty.
    inset = n * 0.08
    tile = (inset, inset, n - inset, n - inset)
    corner = (n - 2 * inset) * 0.22

    accumulator = [[[0, 0, 0, 0] for _ in range(size)] for _ in range(size)]
    for sy in range(n):
        y = sy + 0.5
        for sx in range(n):
            x = sx + 0.5
            colour = None
            if rounded(x, y, *tile, corner):
                colour = TILE
                bar_left = inset + (n - 2 * inset) * BAR_LEFT
                # The tolerance line runs the height of the bars.
                line_x = inset + (n - 2 * inset) * TOLERANCE_AT
                if abs(x - line_x) <= n * 0.008 and inset + (n - 2 * inset) * 0.24 <= y <= inset + (n - 2 * inset) * 0.84:
                    colour = LINE
                for centre, length, bar in BARS:
                    top = inset + (n - 2 * inset) * (centre - BAR_HEIGHT / 2)
                    bottom = inset + (n - 2 * inset) * (centre + BAR_HEIGHT / 2)
                    right = bar_left + (n - 2 * inset) * length
                    if top <= y <= bottom and bar_left <= x <= right:
                        colour = bar
            pixel = accumulator[sy // SUPERSAMPLE][sx // SUPERSAMPLE]
            if colour is not None:
                pixel[0] += colour[0]
                pixel[1] += colour[1]
                pixel[2] += colour[2]
                pixel[3] += 255

    rows = bytearray()
    weight = SUPERSAMPLE * SUPERSAMPLE
    for row in accumulator:
        rows.append(0)
        for r, g, b, a in row:
            # Straight alpha: divide the colour by its own coverage.
            covered = max(a, 1)
            rows.extend((
                min(255, round(r * 255 / covered)),
                min(255, round(g * 255 / covered)),
                min(255, round(b * 255 / covered)),
                round(a / weight),
            ))
    return bytes(rows)


def png(path, size, rows):
    def chunk(tag, data):
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data)))

    with open(path, "wb") as out:
        out.write(b"\x89PNG\r\n\x1a\n")
        out.write(chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)))
        out.write(chunk(b"IDAT", zlib.compress(rows, 9)))
        out.write(chunk(b"IEND", b""))


def main():
    target = sys.argv[1]
    os.makedirs(target, exist_ok=True)
    # The set `iconutil` expects.
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            pixels = size * scale
            suffix = "" if scale == 1 else "@2x"
            rows = draw(pixels)
            png(os.path.join(target, f"icon_{size}x{size}{suffix}.png"), pixels, rows)
            print(f"  icon_{size}x{size}{suffix}.png")


if __name__ == "__main__":
    main()
