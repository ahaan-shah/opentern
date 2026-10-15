#!/usr/bin/env python3
"""Regenerates the Opentern icon.

A "T" for terminal, drawn from a chart: its bar is a rising price line
(teal to violet, softly lit) and its stem a green candlestick with wicks,
on a dark ink tile with a faint dot grid. The tile is the same true
superellipse (n = 5) as Magpie's, so the two apps look like siblings.

packaging/linux/opentern.svg is the icon everywhere on Linux.
opentern-24.svg and opentern-16.svg are simpler variants for tiny sizes (a
bolder T; no grid, glow or shadow). Two rasters are made with resvg (or
rsvg-convert): icon-1024.png for the macOS .icns, and the window icon
embedded in the binary. The Windows opentern.ico holds every size rendered
separately (needs ImageMagick).

Like Magpie's: plain SVG only (paths, gradients, opacity, transforms). No
filters, clip paths or <use>, which some icon loaders ignore; soft shadows
are stacks of faint copies instead of blurs.

    python3 scripts/make-icons.py
"""

import math
import pathlib
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "packaging" / "linux"

C, N = 512.0, 5.0  # centre, superellipse exponent

# On the 1024 grid: the T's bar (a price line) and its stem (a candle:
# wick top, body top, body bottom, wick bottom).
BAR = "M262 326L414 300L540 312L762 264"
STEM = (366, 404, 704, 786)
BODY_W = 112


def squircle(R, samples=96):
    pts = []
    for i in range(samples):
        a = 2 * math.pi * i / samples
        c, s = math.cos(a), math.sin(a)
        x = math.copysign(abs(c) ** (2 / N), c)
        y = math.copysign(abs(s) ** (2 / N), s)
        pts.append((C + R * x, C + R * y))
    # Closed Catmull-Rom spline -> cubic Béziers.
    f = lambda v: f"{v:.2f}".rstrip("0").rstrip(".")
    d = [f"M{f(pts[0][0])} {f(pts[0][1])}"]
    n = len(pts)
    for i in range(n):
        p0, p1, p2, p3 = pts[i - 1], pts[i], pts[(i + 1) % n], pts[(i + 2) % n]
        c1 = (p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6)
        c2 = (p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6)
        d.append(f"C{f(c1[0])} {f(c1[1])} {f(c2[0])} {f(c2[1])} {f(p2[0])} {f(p2[1])}")
    return "".join(d) + "Z"


def rounded(x, y, w, h, r):
    return (
        f"M{x + r} {y}H{x + w - r}Q{x + w} {y} {x + w} {y + r}V{y + h - r}"
        f"Q{x + w} {y + h} {x + w - r} {y + h}H{x + r}Q{x} {y + h} {x} {y + h - r}V{y + r}Q{x} {y} {x + r} {y}Z"
    )


def svg(small=0):
    """small=0 is the master; 1 and 2 are the 24px and 16px variants."""
    r = [412.0, 470.0, 492.0][small]
    tile = squircle(r)
    out = []
    if small == 0:
        # Drop shadow: 14 faint, progressively larger and lower copies.
        for k in range(14, 0, -1):
            out.append(
                f'  <path transform="translate(0 {6 + k * 0.9:.1f})" d="{squircle(r + k * 2.0, 48)}" '
                f'fill="#020611" fill-opacity="0.03"/>'
            )
    out.append(f'  <path d="{tile}" fill="url(#fill)"/>')
    out.append(f'  <path d="{tile}" fill="url(#glow)"/>')
    wt, bt, bb, wb = STEM
    if small == 0:
        # A faint dot grid, like a chart's.
        for gy in range(232, 820, 74):
            for gx in range(232, 820, 74):
                out.append(f'  <circle cx="{gx}" cy="{gy}" r="5" fill="#9DB4FF" fill-opacity="0.07"/>')
        out.append('  <ellipse cx="512" cy="304" rx="300" ry="90" fill="url(#halo)"/>')
        out.append(
            f'  <path d="{BAR}" fill="none" stroke="url(#bar)" stroke-width="64" '
            'stroke-linecap="round" stroke-linejoin="round"/>'
        )
        out.append(f'  <path d="M512 {wt}V{wb}" stroke="#3FE3B4" stroke-width="20" stroke-linecap="round"/>')
        out.append(f'  <path d="{rounded(512 - BODY_W / 2, bt, BODY_W, bb - bt, 18)}" fill="url(#body)"/>')
    else:
        # Tiny sizes: a straight, heavy T that holds at 16 px.
        bar_w, body_w = [0, 104, 120][small], [0, 150, 170][small]
        out.append(
            f'  <path d="M250 300H774" stroke="url(#bar)" stroke-width="{bar_w}" stroke-linecap="round"/>'
        )
        out.append(f'  <path d="{rounded(512 - body_w / 2, 380, body_w, 420, 28)}" fill="url(#body)"/>')
    # Rim light: a stroke on an inset outline, so it sits fully inside the tile.
    out.append(f'  <path d="{squircle(r - 4)}" fill="none" stroke="url(#edge)" stroke-width="8"/>')
    body = "\n".join(out)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>Opentern</title>
  <defs>
    <linearGradient id="fill" x1="164" y1="100" x2="860" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#18213D"/>
      <stop offset="0.55" stop-color="#0D1326"/>
      <stop offset="1" stop-color="#070A16"/>
    </linearGradient>
    <radialGradient id="glow" cx="512" cy="280" r="640" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#6D5BFF" stop-opacity="0.20"/>
      <stop offset="0.7" stop-color="#6D5BFF" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="halo" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#4FD8F0" stop-opacity="0.28"/>
      <stop offset="1" stop-color="#4FD8F0" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="bar" x1="262" y1="0" x2="762" y2="0" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#3EE6D0"/>
      <stop offset="0.55" stop-color="#5B8CFF"/>
      <stop offset="1" stop-color="#A26BFF"/>
    </linearGradient>
    <linearGradient id="body" x1="0" y1="380" x2="0" y2="800" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#5CF2C8"/>
      <stop offset="1" stop-color="#18B98B"/>
    </linearGradient>
    <linearGradient id="edge" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.22"/>
      <stop offset="0.3" stop-color="#FFFFFF" stop-opacity="0"/>
      <stop offset="0.8" stop-color="#000000" stop-opacity="0"/>
      <stop offset="1" stop-color="#000000" stop-opacity="0.35"/>
    </linearGradient>
  </defs>
{body}
</svg>
"""


def render(src, dst, size):
    if shutil.which("resvg"):
        subprocess.run(["resvg", "-w", str(size), "-h", str(size), src, dst], check=True)
    else:
        subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), src, "-o", dst], check=True)


def main():
    """Writes the SVGs the desktop uses, plus the few PNGs that must be raster:
    the macOS .icns source (1024) and the window icon embedded in the binary."""
    OUT.mkdir(parents=True, exist_ok=True)
    assets = ROOT / "crates" / "app" / "assets"
    src = OUT / "opentern.svg"
    src.write_text(svg())
    for small, size in ((1, 24), (2, 16)):
        (OUT / f"opentern-{size}.svg").write_text(svg(small))
    for old in OUT.glob("icon-*.png"):
        old.unlink()
    render(str(src), str(OUT / "icon-1024.png"), 1024)
    render(str(src), str(assets / "icon-512.png"), 512)
    # Windows .ico: each size rendered on its own (the simpler variants at
    # 16-32) so Explorer, the taskbar and Start never scale a bitmap.
    tmp = pathlib.Path(tempfile.mkdtemp())
    layers = []
    for size in (16, 20, 24, 32, 40, 48, 64, 96, 128, 256):
        small = 2 if size <= 20 else 1 if size <= 32 else 0
        svg_path = tmp / f"m{size}.svg"
        svg_path.write_text(svg(small))
        png = tmp / f"m{size}.png"
        render(str(svg_path), str(png), size)
        layers.append(str(png))
    subprocess.run(["magick", *layers, str(assets / "opentern.ico")], check=True)
    shutil.rmtree(tmp)
    print("wrote packaging/linux/opentern.svg, opentern-24.svg, opentern-16.svg, opentern.ico and the raster icons")


if __name__ == "__main__":
    main()
