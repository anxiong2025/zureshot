#!/usr/bin/env python3
from PIL import Image, ImageDraw
import math

size = 44
scale = 4
big_size = size * scale

big = Image.new('RGBA', (big_size, big_size), (0, 0, 0, 0))
draw = ImageDraw.Draw(big)

bcx, bcy = big_size // 2, big_size // 2
br = 18 * scale

# Red circle background
draw.ellipse([bcx - br, bcy - br, bcx + br, bcy + br], fill=(255, 59, 48, 255))

# Draw white infinity symbol using lemniscate parametric curve
a = 11.0 * scale
line_w = int(2.5 * scale)
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
    draw.line([p1, p2], fill=(255, 255, 255, 255), width=line_w)

# Downscale with anti-aliasing
img = big.resize((size, size), Image.LANCZOS)
img.save('tray-recording.png')
print('Generated tray-recording.png')

# Verify output
v = Image.open('tray-recording.png')
for y in range(0, 44):
    row = ''
    for x in range(0, 44):
        p = v.getpixel((x, y))
        if p[3] < 50:
            row += ' '
        elif p[0] > 200 and p[1] > 200:
            row += 'W'
        elif p[0] > 200:
            row += 'R'
        else:
            row += '.'
    print(f'{y:2d}|{row}|')
