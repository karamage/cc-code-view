#!/usr/bin/env python3
"""アプリのアイコンを描く。星座と航路のモチーフ。"""
import math
import os
import subprocess
import sys
import tempfile
from PIL import Image, ImageDraw, ImageFilter

S = 2048
PAD = int(S * 0.098)
BODY = S - PAD * 2
RADIUS = int(BODY * 0.225)

def base():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    grad = Image.new("RGBA", (BODY, BODY))
    px = grad.load()
    for y in range(BODY):
        for x in range(BODY):
            d = math.hypot(x - BODY * 0.62, y - BODY * 0.34) / BODY
            t = min(1.0, d * 1.35)
            px[x, y] = (int(36 - 30 * t), int(36 - 30 * t), int(82 - 70 * t), 255)
    mask = Image.new("L", (BODY, BODY), 0)
    d = ImageDraw.Draw(mask)
    d.rectangle((RADIUS, 0, BODY - RADIUS, BODY), fill=255)
    d.rectangle((0, RADIUS, BODY, BODY - RADIUS), fill=255)
    for cx, cy in ((RADIUS, RADIUS), (BODY - RADIUS, RADIUS), (RADIUS, BODY - RADIUS), (BODY - RADIUS, BODY - RADIUS)):
        d.ellipse((cx - RADIUS, cy - RADIUS, cx + RADIUS, cy + RADIUS), fill=255)
    img.paste(grad, (PAD, PAD), mask)
    return img, mask

def main(out):
    img, mask = base()
    P = lambda x, y: (PAD + BODY * x, PAD + BODY * y)
    stars = [(0.2, 0.72, 34), (0.36, 0.5, 26), (0.56, 0.58, 40), (0.72, 0.3, 62), (0.84, 0.56, 28), (0.42, 0.22, 22)]
    lines = [(0, 1), (1, 2), (2, 3), (3, 4), (1, 5), (5, 3)]
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    for a, b in lines:
        gd.line([P(*stars[a][:2]), P(*stars[b][:2])], fill=(168, 164, 255, 150), width=int(S * 0.008))
    for x, y, r in stars:
        c = P(x, y)
        gd.ellipse((c[0] - r * 2.4, c[1] - r * 2.4, c[0] + r * 2.4, c[1] + r * 2.4), fill=(168, 164, 255, 150))
    glow = glow.filter(ImageFilter.GaussianBlur(S * 0.018))
    img = Image.alpha_composite(img, glow)
    d = ImageDraw.Draw(img)
    for a, b in lines:
        route = (a, b) in [(0, 1), (1, 2), (2, 3)]
        color = (115, 226, 212, 255) if route else (168, 164, 255, 150)
        d.line([P(*stars[a][:2]), P(*stars[b][:2])], fill=color, width=int(S * (0.0085 if route else 0.005)))
    for x, y, r in stars:
        c = P(x, y)
        d.ellipse((c[0] - r, c[1] - r, c[0] + r, c[1] + r), fill=(255, 255, 255, 255))
    # 角の外へはみ出した光を消す
    clip = Image.new("L", (S, S), 0)
    clip.paste(mask, (PAD, PAD))
    img.putalpha(Image.composite(img.getchannel("A"), Image.new("L", (S, S), 0), clip))

    with tempfile.TemporaryDirectory() as tmp:
        iconset = os.path.join(tmp, "AppIcon.iconset")
        os.mkdir(iconset)
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                px = size * scale
                name = f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png"
                img.resize((px, px), Image.LANCZOS).save(os.path.join(iconset, name))
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", out], check=True)
    img.resize((512, 512), Image.LANCZOS).save(out.replace(".icns", ".png"))

main(sys.argv[1])
