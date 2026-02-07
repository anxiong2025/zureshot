#!/usr/bin/env python3
from PIL import Image, ImageDraw
import os

# Create icons directory
icons_dir = os.path.dirname(os.path.abspath(__file__))

# Create a simple red circle icon
size = 32
img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
draw = ImageDraw.Draw(img)
draw.ellipse([4, 4, size-4, size-4], fill=(255, 59, 48, 255))

# Save various sizes
img.save(os.path.join(icons_dir, "32x32.png"))
img.save(os.path.join(icons_dir, "tray.png"))
img.resize((128, 128), Image.Resampling.LANCZOS).save(os.path.join(icons_dir, "128x128.png"))
img.resize((256, 256), Image.Resampling.LANCZOS).save(os.path.join(icons_dir, "128x128@2x.png"))
img.resize((256, 256), Image.Resampling.LANCZOS).save(os.path.join(icons_dir, "icon.icns"))
img.resize((256, 256), Image.Resampling.LANCZOS).save(os.path.join(icons_dir, "icon.ico"))

print("Icons created successfully!")
