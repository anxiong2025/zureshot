#!/usr/bin/env python3
import struct
import zlib
import os

def create_rgba_png(width, height, r, g, b, a=255):
    """Create a valid RGBA PNG file with a solid color."""
    # PNG signature
    signature = b'\x89PNG\r\n\x1a\n'
    
    # IHDR chunk (image header)
    ihdr_data = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA
    ihdr_crc = zlib.crc32(b'IHDR' + ihdr_data) & 0xffffffff
    ihdr = struct.pack('>I', 13) + b'IHDR' + ihdr_data + struct.pack('>I', ihdr_crc)
    
    # IDAT chunk - raw image data with filter bytes
    raw_data = b''
    for y in range(height):
        raw_data += b'\x00'  # filter type none for each row
        for x in range(width):
            raw_data += bytes([r, g, b, a])
    
    compressed = zlib.compress(raw_data, 9)
    idat_crc = zlib.crc32(b'IDAT' + compressed) & 0xffffffff
    idat = struct.pack('>I', len(compressed)) + b'IDAT' + compressed + struct.pack('>I', idat_crc)
    
    # IEND chunk
    iend_crc = zlib.crc32(b'IEND') & 0xffffffff
    iend = struct.pack('>I', 0) + b'IEND' + struct.pack('>I', iend_crc)
    
    return signature + ihdr + idat + iend

def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    os.chdir(script_dir)
    
    # Create icons with a nice red color
    icons = [
        ('32x32.png', 32, 32),
        ('128x128.png', 128, 128),
        ('128x128@2x.png', 256, 256),
        ('tray.png', 22, 22),
    ]
    
    for filename, w, h in icons:
        png_data = create_rgba_png(w, h, 220, 50, 50, 255)  # Red color
        with open(filename, 'wb') as f:
            f.write(png_data)
        print(f'Created {filename} ({w}x{h}) - {len(png_data)} bytes')
    
    # Create minimal .icns and .ico placeholders
    # Tauri primarily uses PNG on macOS
    with open('icon.icns', 'wb') as f:
        f.write(b'')
    with open('icon.ico', 'wb') as f:
        f.write(b'')
    print('Created icon.icns and icon.ico (empty placeholders)')
    
    print('\nDone! All icons created successfully.')

if __name__ == '__main__':
    main()
