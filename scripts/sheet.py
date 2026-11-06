#!/usr/bin/env python3
"""撮影した画像を 1 枚に並べる。使い方: sheet.py <出力> <列数> <画像...>"""
import sys
from PIL import Image
out, cols, files = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
images = [Image.open(f).convert("RGB") for f in files]
w = 1000
h = int(images[0].height * w / images[0].width)
rows = (len(images) + cols - 1) // cols
sheet = Image.new("RGB", (cols * w + (cols - 1) * 8, rows * h + (rows - 1) * 8), (60, 60, 70))
for i, im in enumerate(images):
    sheet.paste(im.resize((w, h)), ((i % cols) * (w + 8), (i // cols) * (h + 8)))
sheet.save(out)
