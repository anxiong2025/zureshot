import math, os, shutil, tempfile
from PIL import Image, ImageDraw, ImageFilter

D = os.path.dirname(os.path.abspath(__file__))

# ── Infinity curve (lemniscate of Bernoulli) ──────────────────
def draw_infinity(draw, cx, cy, rx, ry, thickness, fill):
    pts = []
    for i in range(501):
        t = 2 * math.pi * i / 500
        d = 1 + math.sin(t) ** 2
        pts.append((cx + rx * math.cos(t) / d,
                     cy + ry * math.sin(t) * math.cos(t) / d))
    r = thickness / 2
    for px, py in pts:
        draw.ellipse([px - r, py - r, px + r, py + r], fill=fill)
    for i in range(len(pts) - 1):
        draw.line([pts[i], pts[i + 1]], fill=fill, width=int(thickness))


# ── App icon: frosted glass + infinity ────────────────────────
def make_app_icon(size):
    img = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    m = max(1, int(size * 0.02))        # margin
    cr = int(size * 0.22)               # corner radius
    s = size / 256.0

    # --- 1) Frosted glass base plate ---
    # Soft gradient from top-white to bottom-light-gray
    base = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    bd = ImageDraw.Draw(base)
    # Draw filled rounded rect as mask
    mask = Image.new('L', (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [m, m, size - m - 1, size - m - 1], radius=cr, fill=255)

    # Vertical gradient: top (#f8f9fc) -> bottom (#e8eaf0)
    for y in range(size):
        t = y / max(1, size - 1)
        r_c = int(248 + (232 - 248) * t)
        g_c = int(249 + (234 - 249) * t)
        b_c = int(252 + (240 - 252) * t)
        bd.line([(0, y), (size - 1, y)], fill=(r_c, g_c, b_c, 255))
    base.putalpha(mask)
    img = Image.alpha_composite(img, base)

    # --- 2) Subtle inner highlight (top edge glow) ---
    highlight = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    hl_h = int(size * 0.35)
    for i in range(hl_h):
        t = i / max(1, hl_h)
        alpha = int(40 * (1 - t) ** 2)
        yy = m + i
        ImageDraw.Draw(highlight).line(
            [(m, yy), (size - m - 1, yy)],
            fill=(255, 255, 255, alpha))
    highlight.putalpha(ImageDraw.Draw(
        Image.new('L', (size, size), 0)).rounded_rectangle(
        [m, m, size - m - 1, size - m - 1], radius=cr, fill=255) or
        Image.new('L', (size, size), 0))
    # Clip highlight to base shape
    hl_mask = Image.new('L', (size, size), 0)
    ImageDraw.Draw(hl_mask).rounded_rectangle(
        [m, m, size - m - 1, size - m - 1], radius=cr, fill=255)
    ha = highlight.split()[3]
    ha_d = list(ha.getdata())
    hm_d = list(hl_mask.getdata())
    highlight.putalpha(Image.new('L', (size, size)).point(
        lambda _: 0))
    hl_final = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    for i in range(hl_h):
        t = i / max(1, hl_h)
        alpha = int(40 * (1 - t) ** 2)
        yy = m + i
        ImageDraw.Draw(hl_final).line(
            [(m, yy), (size - m - 1, yy)],
            fill=(255, 255, 255, alpha))
    cm = Image.new('L', (size, size), 0)
    cm_data = [min(a, b) for a, b in zip(
        list(hl_final.split()[3].getdata()), hm_d)]
    cm.putdata(cm_data)
    hl_final.putalpha(cm)
    img = Image.alpha_composite(img, hl_final)

    # --- 3) Subtle border (thin glass edge) ---
    border = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    bdr = ImageDraw.Draw(border)
    # Outer stroke: very subtle dark outline
    bdr.rounded_rectangle(
        [m, m, size - m - 1, size - m - 1],
        radius=cr, outline=(0, 0, 0, 20), width=max(1, int(s)))
    # Inner bright edge
    inner_m = m + max(1, int(s))
    inner_cr = max(1, cr - max(1, int(s)))
    bdr.rounded_rectangle(
        [inner_m, inner_m, size - inner_m - 1, size - inner_m - 1],
        radius=inner_cr, outline=(255, 255, 255, 50), width=max(1, int(s)))
    img = Image.alpha_composite(img, border)

    # --- 4) Bottom shadow ---
    shadow = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    sh_h = int(size * 0.05)
    for i in range(sh_h):
        t = i / max(1, sh_h)
        alpha = int(15 * (1 - t))
        yy = size - m - 1 - i
        if yy > m + cr:
            ImageDraw.Draw(shadow).line(
                [(m + cr, yy), (size - m - cr, yy)],
                fill=(0, 0, 0, alpha))
    sha = shadow.split()[3]
    sd = list(sha.getdata())
    md = list(hl_mask.getdata())
    cm2 = Image.new('L', (size, size), 0)
    cm2.putdata([min(a, b) for a, b in zip(sd, md)])
    shadow.putalpha(cm2)
    img = Image.alpha_composite(img, shadow)

    # --- 5) Infinity symbol (supersampled) ---
    ss = 2
    big = Image.new('RGBA', (size * ss, size * ss), (0, 0, 0, 0))
    cx, cy = size * ss / 2, size * ss / 2
    rx = 72 * s * ss
    ry = 42 * s * ss
    th = max(2, int(22 * s * ss))
    # Dark graphite color
    ink = (45, 55, 72, 230)
    draw_infinity(ImageDraw.Draw(big), cx, cy, rx, ry, th, ink)
    smooth = big.resize((size, size), Image.LANCZOS)
    img = Image.alpha_composite(img, smooth)

    return img


# ── Tray icon ─────────────────────────────────────────────────
def make_tray_icon(size=44, recording=False):
    img = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    s = size / 44.0
    m = max(1, int(2 * s))          # margin
    cr = int(size * 0.22)           # corner radius

    # White rounded-rect background
    draw = ImageDraw.Draw(img)
    draw.rounded_rectangle(
        [m, m, size - m - 1, size - m - 1],
        radius=cr, fill=(255, 255, 255, 255))

    # Subtle border for contrast on light menu bars
    draw.rounded_rectangle(
        [m, m, size - m - 1, size - m - 1],
        radius=cr, outline=(0, 0, 0, 30), width=max(1, int(s)))

    # Draw infinity symbol (supersampled)
    ss = 2
    big = Image.new('RGBA', (size * ss, size * ss), (0, 0, 0, 0))
    cx, cy = size * ss / 2, size * ss / 2
    draw_infinity(ImageDraw.Draw(big),
                  cx, cy, 13 * s * ss, 8 * s * ss,
                  max(2, int(4.2 * s * ss)), (50, 50, 50, 255))
    smooth = big.resize((size, size), Image.LANCZOS)
    img = Image.alpha_composite(img, smooth)

    if recording:
        # Red dot at top-right corner
        ImageDraw.Draw(img).ellipse(
            [31 * s, 3 * s, 41 * s, 13 * s],
            fill=(255, 59, 48, 255))

    return img


# ── Generate ──────────────────────────────────────────────────
print('Generating Zureshot icons (frosted glass + infinity)...')

make_tray_icon(44, False).save(os.path.join(D, 'tray.png'))
make_tray_icon(44, True).save(os.path.join(D, 'tray-recording.png'))
print('  tray icons done')

for sz in [32, 128, 256]:
    make_app_icon(sz).save(os.path.join(D, f'{sz}x{sz}.png'))
    print(f'  {sz}x{sz}.png done')

shutil.copy(os.path.join(D, '256x256.png'), os.path.join(D, '128x128@2x.png'))

# .icns
iset = tempfile.mkdtemp(suffix='.iconset')
for sd, sf in [(16,'16x16'),(32,'16x16@2x'),(32,'32x32'),(64,'32x32@2x'),
               (128,'128x128'),(256,'128x128@2x'),(256,'256x256'),
               (512,'256x256@2x')]:
    make_app_icon(sd).save(os.path.join(iset, f'icon_{sf}.png'))
os.system(f'iconutil -c icns "{iset}" -o "{os.path.join(D, "icon.icns")}"')

# .ico
Image.open(os.path.join(D, '256x256.png')).save(
    os.path.join(D, 'icon.ico'), format='ICO',
    sizes=[(32,32),(48,48),(64,64),(128,128),(256,256)])
print('All icons generated!')
