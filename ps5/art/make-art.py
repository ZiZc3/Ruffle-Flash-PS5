#!/usr/bin/env python3
# Ruffle Flash PS5: the launcher art and the in-app logo, made from Ruffle's
# own logo (favicon-180.png from ruffle.rs) recoloured to the app's orange.
#   icon-source.png        1024x1024, logo on black      -> sce_sys/icon0.png
#   background-source.png  3840x2160, ember gradient with a warm glow on the
#                          right and the app's light waves -> sce_sys/pic0/pic1.dds
#   logo.png               512x512, transparent          -> assets/logo.png
# The PS5 draws the app's name and Play button over the left of the
# background, so the logo sits on the right (as XPSemu's art does).
#
# Usage: make-art.py SOURCE_LOGO OUTPUT_DIR
import math
import random
import sys

import numpy as np
from PIL import Image, ImageFilter

src = sys.argv[1]
out = sys.argv[2] if len(sys.argv) > 2 else '.'

ORANGE_LIGHT = np.array([255, 150, 58], np.float32)
ORANGE_DEEP = np.array([232, 88, 22], np.float32)


def logo(size):
    """The logo at `size` px, recoloured orange with its fold shading, RGBA."""
    im = Image.open(src).convert('RGBA')
    a = np.asarray(im, np.float32)
    alpha = a[..., 3] / 255
    # Shading relative to the logo's flat yellow (255,173,51): 1 on the
    # face, ~0.57 in the fold.
    shade = np.where(alpha > 0.05, a[..., 1] / np.maximum(alpha, 0.05) / 173.0, 1.0)
    shade = np.clip(shade, 0, 1.1)

    big = 8 * im.width
    al = Image.fromarray((alpha * 255).astype(np.uint8)).resize((big, big), Image.LANCZOS)
    sh = Image.fromarray((shade / 1.1 * 255).astype(np.uint8)).resize((big, big), Image.BICUBIC)
    al = np.asarray(al, np.float32) / 255
    # Crisp edges: sharpen the upscaled (soft) alpha around its midpoint.
    t = np.clip((al - 0.38) / 0.24, 0, 1)
    al = t * t * (3 - 2 * t)
    sh = np.asarray(sh, np.float32) / 255 * 1.1

    y, x = np.mgrid[0:big, 0:big].astype(np.float32) / big
    g = np.clip((x * 0.4 + y * 0.6), 0, 1)[..., None]
    body = ORANGE_LIGHT * (1 - g) + ORANGE_DEEP * g
    rgb = body * sh[..., None]

    rgba = np.zeros((big, big, 4), np.float32)
    rgba[..., :3] = np.clip(rgb, 0, 255)
    rgba[..., 3] = al * 255
    img = Image.fromarray(rgba.astype(np.uint8), 'RGBA')
    # Trim to the logo itself, keep it square, then scale down (antialiased).
    bbox = img.getbbox()
    side = max(bbox[2] - bbox[0], bbox[3] - bbox[1])
    sq = Image.new('RGBA', (side, side), (0, 0, 0, 0))
    sq.alpha_composite(img.crop(bbox), ((side - (bbox[2] - bbox[0])) // 2, (side - (bbox[3] - bbox[1])) // 2))
    return sq.resize((size, size), Image.LANCZOS)


def glow(img, radius, color, strength):
    m = img.split()[3].filter(ImageFilter.GaussianBlur(radius))
    g = np.asarray(m, np.float32) / 255
    out = np.zeros((img.height, img.width, 4), np.float32)
    out[..., :3] = color
    out[..., 3] = np.clip(g * strength, 0, 1) * 255
    return Image.fromarray(out.astype(np.uint8), 'RGBA')


def radial(w, h, cx, cy, radius, inner, outer):
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    d = np.clip(np.sqrt((x - cx) ** 2 + (y - cy) ** 2) / radius, 0, 1)[..., None] ** 1.3
    return np.array(inner, np.float32) * (1 - d) + np.array(outer, np.float32) * d


def make_icon():
    size = 1024
    bg = radial(size, size, size / 2, size * 0.46, size * 0.75, (38, 16, 6), (0, 0, 0))
    icon = Image.fromarray(bg.astype(np.uint8), 'RGB').convert('RGBA')
    lg = logo(int(size * 0.66))
    layer = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    layer.alpha_composite(lg, ((size - lg.width) // 2, (size - lg.height) // 2))
    icon.alpha_composite(glow(layer, size / 22, (242, 107, 29), 0.55))
    icon.alpha_composite(layer)
    icon.convert('RGB').save(f'{out}/icon-source.png')


def waves(w, h, t):
    """The app's light ribbons (ui/gfx.rs draw_waves) at this size, RGBA."""
    s = w / 1920
    ribbons = [(610, 46, 0.0021, 0.32, 150, 0.0), (660, 60, 0.0016, -0.22, 190, 2.1),
               (720, 38, 0.0027, 0.41, 120, 4.2)]
    a = np.zeros((h, w), np.float32)
    ys = np.arange(h, dtype=np.float32)[:, None]
    xs = np.arange(w, dtype=np.float32)[None, :] / s
    for base, amp, k, speed, thick, phase in ribbons:
        yc = (base + amp * np.sin(xs * k + t * speed + phase)
              + amp * 0.35 * np.sin(xs * k * 2.3 - t * speed * 1.7 + phase)) * s
        th = (thick + 40 * np.sin(xs * k * 0.7 + t * 0.15 + phase)) * s
        edge = np.clip(1.6 - np.abs(ys - yc) / s, 0, 1) * 0.30
        below = (ys - yc) / th
        body = np.where((below > 0) & (below < 1), 0.085 * (1 - below), 0)
        a = 1 - (1 - a) * (1 - edge) * (1 - body)
    img = np.zeros((h, w, 4), np.float32)
    img[..., :3] = (255, 210, 168)
    img[..., 3] = a * 255
    return Image.fromarray(img.astype(np.uint8), 'RGBA')


def make_background():
    w, h = 3840, 2160
    lx, ly = int(w * 0.72), int(h * 0.42)
    # The app's ember gradient, warmer around the logo.
    y = np.linspace(0, 1, h, dtype=np.float32)[:, None, None]
    grad = np.array([42, 18, 8], np.float32) * (1 - y) + np.array([8, 6, 10], np.float32) * y
    grad = np.broadcast_to(grad, (h, w, 3)).copy()
    warm = radial(w, h, lx, ly, w * 0.5, (120, 46, 14), (0, 0, 0))
    img = Image.fromarray(np.clip(grad + warm * 0.55, 0, 255).astype(np.uint8), 'RGB').convert('RGBA')

    img.alpha_composite(waves(w, h, 3.0))

    # Motes of light, as embers.
    random.seed(11)
    motes = Image.new('RGBA', (w, h), (0, 0, 0, 0))
    from PIL import ImageDraw
    d = ImageDraw.Draw(motes)
    for _ in range(220):
        mx, my = random.uniform(w * 0.3, w), random.uniform(0, h)
        r = random.uniform(2, 6)
        d.ellipse((mx - r, my - r, mx + r, my + r), fill=(255, 170, 90, random.randint(30, 130)))
    img.alpha_composite(motes.filter(ImageFilter.GaussianBlur(1.5)))

    # Darker on the left, where the PS5 puts the name and Play button.
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    shade = np.clip(1 - xx / (w * 0.55), 0, 1) ** 1.5 * 0.55
    a = np.asarray(img, np.float32)
    a[..., :3] *= (1 - shade)[..., None]
    Image.fromarray(a.astype(np.uint8), 'RGBA').convert('RGB').save(f'{out}/background-source.png')


logo(512).save(f'{out}/logo.png')
make_icon()
make_background()
print('art written to', out)
