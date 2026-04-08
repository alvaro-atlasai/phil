#!/usr/bin/env python3
"""Generate social preview card for GitHub (1280x640)."""
from PIL import Image, ImageDraw, ImageFont
import os

W, H = 1280, 640
BG = (17, 17, 27)       # Catppuccin Mocha base
FG = (205, 214, 244)    # Catppuccin text
ACCENT = (137, 180, 250) # Catppuccin blue
DIM = (108, 112, 134)   # Catppuccin overlay0
GREEN = (166, 227, 161)  # Catppuccin green

img = Image.new("RGB", (W, H), BG)
draw = ImageDraw.Draw(img)

# Try to find a good monospace font
font_paths = [
    "/System/Library/Fonts/SFMono-Regular.otf",
    "/System/Library/Fonts/Menlo.ttc",
    "/System/Library/Fonts/Monaco.ttf",
    "/Library/Fonts/SF-Mono-Regular.otf",
]
font_path = None
for p in font_paths:
    if os.path.exists(p):
        font_path = p
        break

def font(size):
    if font_path:
        return ImageFont.truetype(font_path, size)
    return ImageFont.load_default()

# Draw a subtle terminal-style top bar
bar_h = 44
draw.rectangle([0, 0, W, bar_h], fill=(30, 30, 46))
# Traffic light dots
for i, color in enumerate([(243, 139, 168), (249, 226, 175), (166, 227, 161)]):
    draw.ellipse([24 + i*28, 14, 24 + i*28 + 16, 14 + 16], fill=color)

# Title
title_font = font(72)
draw.text((W//2, 170), "phil", fill=FG, font=title_font, anchor="mm")

# Tagline
tag_font = font(32)
draw.text((W//2, 240), "Pipe anything through AI.", fill=ACCENT, font=tag_font, anchor="mm")

# Terminal example
code_font = font(24)
y = 320
lines = [
    (DIM, "$ "),
    (FG, "curl api.example.com/users "),
    (DIM, "| "),
    (GREEN, "phil "),
    (FG, '"names of active users"'),
]

# Draw the command line by segments
x = 160
for color, text in lines:
    bbox = draw.textbbox((x, y), text, font=code_font)
    draw.text((x, y), text, fill=color, font=code_font)
    x = bbox[2]

# Output line
draw.text((160, y + 44), "Alice, Carol, Diana", fill=DIM, font=code_font)

# Second example
y2 = y + 120
x = 160
lines2 = [
    (DIM, "$ "),
    (FG, "git diff --staged "),
    (DIM, "| "),
    (GREEN, "phil "),
    (FG, "@commit"),
]
for color, text in lines2:
    bbox = draw.textbbox((x, y2), text, font=code_font)
    draw.text((x, y2), text, fill=color, font=code_font)
    x = bbox[2]

draw.text((160, y2 + 44), "feat(auth): add JWT session tokens", fill=DIM, font=code_font)

# Footer
footer_font = font(18)
draw.text((W//2, H - 40), "Local Phi-4 · ~160ms · Zero config · Single Rust binary", fill=DIM, font=footer_font, anchor="mm")

out = os.path.join(os.path.dirname(os.path.dirname(__file__)), "social-preview.png")
img.save(out, "PNG")
print(f"Saved {out} ({os.path.getsize(out) // 1024}KB)")
