#!/usr/bin/env python3
"""Regenerates the Opentern icon.

A terminal prompt and a chart: a ">" chevron (teal to blue) and three rising
candlesticks on a dark ink tile, the last one bright and glowing like a
blinking cursor. Faint grid lines, as on a chart. The tile is the same true
superellipse (n = 5) as Magpie's, so the two apps look like siblings.

packaging/linux/opentern.svg is the icon everywhere on Linux.
opentern-24.svg and opentern-16.svg are simpler variants for tiny sizes (the
chevron and the bright candle only, bigger; no grid, glow or shadow). Two
rasters are made with resvg (or rsvg-convert): icon-1024.png for the macOS
.icns, and the window icon embedded in the binary. The Windows opentern.ico
holds every size rendered separately (needs ImageMagick).

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

# On the 1024 grid. The prompt chevron, and the candles as
# (x, wick top, body top, body bottom, wick bottom).
CHEVRON = "M232 398L346 512L232 626"
CANDLES = [(486, 548, 584, 690, 724), (616, 430, 468, 618, 654), (752, 272, 316, 540, 580)]
CANDLE_W = 78
DIM = ["#56698E", "#8094BC"]  # the two earlier candles


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


def candle(c, body, wick, wick_w=14):
    x, wt, bt, bb, wb = c
    w = CANDLE_W
    return (
        f'    <path d="M{x} {wt}V{wb}" stroke="{wick}" stroke-width="{wick_w}" stroke-linecap="round"/>\n'
        f'    <path d="{rounded(x - w / 2, bt, w, bb - bt, 10)}" fill="{body}"/>'
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
    if small == 0:
        for y in (330, 512, 694):
            out.append(f'  <path d="M190 {y}H834" stroke="#8FB6FF" stroke-opacity="0.07" stroke-width="4"/>')
        out.append("  <g>")
        out.append(
            f'    <path d="{CHEVRON}" fill="none" stroke="url(#chev)" stroke-width="60" '
            'stroke-linecap="round" stroke-linejoin="round"/>'
        )
        for c, col in zip(CANDLES[:2], DIM):
            out.append(candle(c, col, col))
        last = CANDLES[2]
        out.append(f'    <ellipse cx="{last[0]}" cy="{(last[2] + last[3]) / 2}" rx="150" ry="210" fill="url(#halo)"/>')
        out.append(candle(last, "url(#cur)", "#7FF5DD"))
    else:
        # Tiny sizes: the chevron and the bright candle, bigger and bolder.
        zoom = [1.0, 1.22, 1.3][small]
        tx = 512 - 492 * zoom
        ty = 512 - 448 * zoom
        out.append(f'  <g transform="translate({tx:.1f} {ty:.1f}) scale({zoom})">')
        out.append(
            f'    <path d="M232 340L372 480L232 620" fill="none" stroke="url(#chev)" stroke-width="{[0, 84, 100][small]}" '
            'stroke-linecap="round" stroke-linejoin="round"/>'
        )
        out.append(candle((640, 250, 290, 600, 640), "url(#cur)", "#7FF5DD", [0, 22, 26][small]))
    out.append("  </g>")
    # Rim light: a stroke on an inset outline, so it sits fully inside the tile.
    out.append(f'  <path d="{squircle(r - 4)}" fill="none" stroke="url(#edge)" stroke-width="8"/>')
    body = "\n".join(out)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>Opentern</title>
  <defs>
    <linearGradient id="fill" x1="164" y1="100" x2="860" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#1A2540"/>
      <stop offset="0.6" stop-color="#0E1528"/>
      <stop offset="1" stop-color="#080C18"/>
    </linearGradient>
    <radialGradient id="glow" cx="300" cy="170" r="720" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#4C7DFF" stop-opacity="0.22"/>
      <stop offset="0.7" stop-color="#4C7DFF" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="chev" x1="0" y1="340" x2="0" y2="640" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#5EEAD4"/>
      <stop offset="1" stop-color="#3B82F6"/>
    </linearGradient>
    <radialGradient id="halo" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#2EE6C5" stop-opacity="0.30"/>
      <stop offset="0.55" stop-color="#2EE6C5" stop-opacity="0.08"/>
      <stop offset="1" stop-color="#2EE6C5" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="cur" x1="0" y1="290" x2="0" y2="600" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#5CF2D6"/>
      <stop offset="1" stop-color="#1FB89A"/>
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
