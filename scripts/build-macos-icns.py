#!/usr/bin/env python3
"""Rebuild src-tauri/icons/icon.icns in the macOS app-icon shape.

macOS does not mask a legacy .icns — whatever silhouette the file contains is
what the Dock draws. Our artwork is a full-bleed square, so it sat among the
system icons with hard 90-degree corners while everything beside it was a
rounded squircle.

The geometry here is measured, not guessed: the alpha channel of the stock
Notes.app and Reminders.app icons both put the icon body at exactly 824x824
inside a 1024x1024 canvas — a 100px margin on all four sides, centred. Their
corner is Apple's continuous-curvature rounded rect, which no single
superellipse reproduces exactly — the implied exponent drifts from ~4.9
mid-arc to ~5.8 near the tangents. Sweeping the exponent against that measured
profile puts the worst-case silhouette error at its minimum at n=5.44: 13px at
1024, i.e. ~3px at the size the Dock actually draws.

Only icon.icns is rebuilt. icon.ico and the Square*Logo.png set are Windows
assets, where icons are full-bleed by convention and the OS supplies no mask —
insetting them would just render the app smaller than its neighbours there.

Usage:  python3 scripts/build-macos-icns.py
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:  # pragma: no cover - developer tooling
    sys.exit("Pillow is required: python3 -m pip install --user Pillow")

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "src-tauri" / "icons" / "icon.png"
TARGET = ROOT / "src-tauri" / "icons" / "icon.icns"

CANVAS = 1024
BODY = 824
EXPONENT = 5.44
# Supersample the mask, then downscale: a superellipse drawn directly at 1024
# has visibly stepped edges where the curve runs nearly parallel to an axis.
SS = 4

# The stock icons carry a soft contact shadow. Without one ours reads as
# flatter than its neighbours in the Dock. Measured off Notes.app: a low
# single-digit-percent spread reaching ~15px out, biased downward.
SHADOW_BLUR = 12
SHADOW_OFFSET = 6
SHADOW_ALPHA = 38


def squircle_mask(size: int, exponent: float) -> Image.Image:
    """An |x|^n + |y|^n = 1 superellipse filling `size`, antialiased."""
    hi = size * SS
    mask = Image.new("L", (hi, hi), 0)
    draw = ImageDraw.Draw(mask)
    half = hi / 2.0
    for py in range(hi):
        # Sample the row through its centre so the span is symmetric.
        v = abs((py + 0.5) - half) / half
        if v >= 1.0:
            continue
        # Solve |u|^n = 1 - |v|^n for the half-width at this row.
        u = (1.0 - v**exponent) ** (1.0 / exponent)
        dx = u * half
        draw.line([(half - dx, py), (half + dx, py)], fill=255)
    return mask.resize((size, size), Image.LANCZOS)


def build_master() -> Image.Image:
    art = Image.open(SOURCE).convert("RGBA").resize((BODY, BODY), Image.LANCZOS)
    mask = squircle_mask(BODY, EXPONENT)
    # Multiply into any alpha the artwork already had rather than replacing it.
    art.putalpha(Image.composite(art.split()[3], Image.new("L", art.size, 0), mask))

    canvas = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    inset = (CANVAS - BODY) // 2

    shadow = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    shadow.paste((0, 0, 0, SHADOW_ALPHA), (inset, inset + SHADOW_OFFSET), mask)
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(SHADOW_BLUR)))
    canvas.alpha_composite(art, (inset, inset))
    return canvas


def main() -> None:
    if not shutil.which("iconutil"):
        sys.exit("iconutil not found — this script only runs on macOS.")

    master = build_master()
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "icon.iconset"
        iconset.mkdir()
        for pt in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                px = pt * scale
                suffix = "" if scale == 1 else "@2x"
                master.resize((px, px), Image.LANCZOS).save(
                    iconset / f"icon_{pt}x{pt}{suffix}.png"
                )
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(TARGET)], check=True
        )
    print(f"wrote {TARGET.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
