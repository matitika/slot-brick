"""Run: python3 tools/brick_sticker.py assets/sticker.png assets/sticker_brick.png

Builds assets/sticker_brick.png from upstream's assets/sticker.png: the SP sticker's own
grain and smudged serial panel, its bottom notch filled so the label is a plain rectangle like
the Brick's, then worn: scuffed edges, nicks, scratches and greasy smudges."""
import sys, math, random
import numpy as np
from PIL import Image, ImageFilter, ImageDraw

src, out = sys.argv[1], sys.argv[2]
im = np.array(Image.open(src).convert('RGBA')).astype(np.float32)
h, w = im.shape[:2]
rng = random.Random(3040)
nrng = np.random.default_rng(3040)

# 1. fill the notch: interior from 50 rows up, the soft bottom edge from beside the notch
filled = im.copy()
for y in range(150, h):
    for x in range(150, 510):
        if im[y, x, 3] >= 250:
            continue
        if y < h - 7:
            filled[y, x] = im[y - 50, x]
        else:
            filled[y, x] = im[y, 20 + (x - 150) % 160]
im = filled

rgb, alpha = im[..., :3], im[..., 3]
yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
edge = np.minimum.reduce([xx, yy, w - 1 - xx, h - 1 - yy])
black = rgb.mean(axis=2) < 90

def layer():
    return Image.new('L', (w, h), 0)

# 2. scuffs: the edges rubbed pale, unevenly, more toward the corners
scuff = np.zeros((h, w), np.float32)
band = np.clip(1 - edge / 9.0, 0, 1) ** 1.6
along = np.zeros((h, w), np.float32)
for _ in range(90):
    # rubbed patches strung along the border
    side = rng.choice('tblr')
    t = rng.random()
    cx, cy = {'t': (t * w, 0), 'b': (t * w, h - 1), 'l': (0, t * h), 'r': (w - 1, t * h)}[side]
    r = rng.uniform(6, 26)
    along += np.exp(-(((xx - cx) ** 2 + (yy - cy) ** 2) / (2 * r * r))) * rng.uniform(0.3, 1)
corner = np.exp(-np.minimum.reduce([
    np.hypot(xx, yy), np.hypot(w - xx, yy), np.hypot(xx, h - yy), np.hypot(w - xx, h - yy)]) / 30)
fibres = np.array(Image.fromarray((nrng.random((h, w)) * 255).astype(np.uint8))
                  .filter(ImageFilter.GaussianBlur(0.7))).astype(np.float32) / 255
scuff = np.clip(band * (0.35 * np.clip(along, 0, 1.4) + 0.5 * corner) * (0.55 + 0.9 * fibres), 0, 0.85)

# 3. nicks: small bites out of the edge where the label lifted
nick = layer(); d = ImageDraw.Draw(nick)
for _ in range(16):
    side = rng.choice('tblr'); t = rng.uniform(0.04, 0.96)
    cx, cy = {'t': (t * w, -1), 'b': (t * w, h), 'l': (-1, t * h), 'r': (w, t * h)}[side]
    rx, ry = rng.uniform(1.5, 4.5), rng.uniform(1.0, 2.6)
    if side in 'lr':
        rx, ry = ry, rx
    d.ellipse([cx - rx, cy - ry, cx + rx, cy + ry], fill=255)
nick = np.array(nick.filter(ImageFilter.GaussianBlur(0.6))).astype(np.float32) / 255

# 4. scratches: thin pale lines, mostly short, a few long, some across the panel
scr = layer(); d = ImageDraw.Draw(scr)
for i in range(16):
    long_one = i < 3
    x0, y0 = rng.uniform(0, w), rng.uniform(0, h)
    ang = rng.uniform(-0.5, 0.5) + rng.choice([0, math.pi, 0.15])
    length = rng.uniform(50, 140) if long_one else rng.uniform(5, 22)
    pts = []
    for k in range(9):
        f = k / 8
        pts.append((x0 + math.cos(ang) * length * f + rng.uniform(-0.6, 0.6),
                    y0 + math.sin(ang) * length * f + rng.uniform(-0.6, 0.6)))
    d.line(pts, fill=rng.randint(40, 90) if long_one else rng.randint(50, 120), width=1)
scr = np.array(scr.filter(ImageFilter.GaussianBlur(0.35))).astype(np.float32) / 255

# 5. smudges: soft greasy patches, faint on black, grey on the white panel
smudge = np.zeros((h, w), np.float32)
for _ in range(14):
    cx, cy = rng.uniform(0, w), rng.uniform(0, h)
    rx, ry = rng.uniform(14, 60), rng.uniform(8, 30)
    smudge += np.exp(-(((xx - cx) / rx) ** 2 + ((yy - cy) / ry) ** 2)) * rng.uniform(0.25, 1)
thumb = np.array(Image.fromarray((nrng.random((h, w)) * 255).astype(np.uint8))
                 .filter(ImageFilter.GaussianBlur(2.5))).astype(np.float32) / 255
smudge = np.clip(smudge, 0, 1) * (0.4 + 1.2 * (thumb - 0.5) + 0.6)

pale = np.array([150, 146, 140], np.float32)
dirt = np.array([120, 116, 110], np.float32)
out_rgb = rgb.copy()
k = scuff[..., None] * black[..., None]
out_rgb = out_rgb * (1 - k) + pale * k
k = (0.55 * scr)[..., None] * black[..., None]
out_rgb = out_rgb * (1 - k) + np.array([205, 202, 196], np.float32) * k
k = (0.30 * scr)[..., None] * (~black)[..., None]
out_rgb = out_rgb * (1 - k) + np.array([150, 150, 150], np.float32) * k
k = np.clip(0.07 * smudge, 0, 0.12)[..., None] * black[..., None]
out_rgb = out_rgb * (1 - k) + pale * k
k = np.clip(0.10 * smudge, 0, 0.16)[..., None] * (~black)[..., None]
out_rgb = out_rgb * (1 - k) + dirt * k
out_a = alpha * (1 - nick)

res = np.dstack([np.clip(out_rgb, 0, 255), np.clip(out_a, 0, 255)]).astype(np.uint8)
Image.fromarray(res, 'RGBA').save(out, optimize=True)
