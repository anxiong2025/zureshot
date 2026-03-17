#!/usr/bin/env python3
"""Generate tray.png: rounded rectangle outline + ∞ symbol on transparent background.
macOS template icons use alpha for visibility and luminance for the mask."""
from PIL import Image, ImageDraw
import math

size = 44
scale = 4
big_size = size * scale

big = Image.new('RGBA', (big_size, big_size), (0, 0, 0, 0))
draw = ImageDraw.Draw(big)

# Rounded rectangle outline (not filled)
margin = 3 * scale
radius = 8 * scale
line_w = int(1.8 * scale)
x0, y0 = margin, margin
x1, y1 = big_size - margin, big_size - margin
color = (0, 0, 0, 255)

# Draw rounded rect outline using arcs + lines
draw.rounded_rectangle([x0, y0, x1, y1], radius=radius, outline=color, width=line_w)

# Draw ∞ (lemniscate) symbol centered inside
bcx, bcy = big_size // 2, big_size // 2
a = 10.0 * scale  # size of the lemniscate
inf_line_w = int(2.2 * scale)

points = []
for i in range(400):
    t = 2 * math.pi * i / 400
    denom = 1 + math.sin(t) ** 2
    x = a * math.cos(t) / denom + bcx
    y = a * math.sin(t) * math.cos(t) / denom + bcy
    points.append((x, y))

for i in range(len(points)):
    p1 = points[i]
    p2 = points[(i + 1) % len(points)]
    draw.line([p1, p2], fill=color, width=inf_line_w)

# Downscale with anti-aliasing
img = big.resize((size, size), Image.LANCZOS)
img.save('tray.png')
print('Generated tray.png (rounded rect outline + infinity symbol)')

# Verify
v = Image.open('tray.png')
for y in range(0, 44):
    row = ''
    for x in range(0, 44):
        r, g, b, a = v.getpixel((x, y))
        if a < 30:
            row += ' '
        elif a < 100:
            row += '.'
        elif a < 180:
            row += '+'
        else:
            row += '#'
    print(f'{y:2d}|{row}|')
