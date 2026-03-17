#!/usr/bin/env python3
"""Generate a CleanShot X-style stop recording tray icon (44x44)."""
from PIL import Image, ImageDraw

size = 44
img = Image.new('RGBA', (size, size), (0, 0, 0, 0))
draw = ImageDraw.Draw(img)

# Filled red circle
center = size // 2
radius = 18
draw.ellipse(
    [center - radius, center - radius, center + radius, center + radius],
    fill=(255, 59, 48, 255)  # macOS system red
)

# White rounded-square stop symbol
sq_half = 7
sq_radius = 3
draw.rounded_rectangle(
    [center - sq_half, center - sq_half, center + sq_half, center + sq_half],
    radius=sq_radius,
    fill=(255, 255, 255, 255)
)

out_path = __import__('os').path.join(__import__('os').path.dirname(__file__), 'tray-recording.png')
img.save(out_path)
print(f'Created {out_path} ({size}x{size})')
