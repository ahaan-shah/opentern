#!/usr/bin/env python3
"""Regenerates the Opentern icon.

Two candlesticks in white line-art, one hollow and one filled, on an indigo
tile: as simple as Magpie's bird, and its sibling. The tile is the same
true superellipse (n = 5) with the same light (a soft glow at the top left,
a rim light, a faint drop shadow) and the mark casts the same soft shadow.

packaging/linux/opentern.svg is the icon everywhere on Linux.
opentern-24.svg and opentern-16.svg are simpler variants for tiny sizes (a
bigger, bolder mark; no shadows). Two rasters are made with resvg (or
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
INK = "#0A1240"  # the tile's shadow colour


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


def mark(color, stroke=44.0):
    """The two candles: a short hollow one, a tall filled one."""
    line = f'stroke="{color}" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round"'
    h = stroke / 2
    return (
        f'<path d="M392 300V400M392 640V724" {line} fill="none"/>'
        f'<path d="{rounded(330, 400, 124, 240, 26)}" {line} fill="none"/>'
        f'<path d="M632 236V330M632 690V788" {line} fill="none"/>'
        f'<path d="{rounded(570 - h, 330 - h, 124 + stroke, 360 + stroke, 34)}" fill="{color}"/>'
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
                f'fill="{INK}" fill-opacity="0.025"/>'
            )
    out.append(f'  <path d="{tile}" fill="url(#fill)"/>')
    out.append(f'  <path d="{tile}" fill="url(#glow)"/>')
    # Rim light: a stroke on an inset outline, so it sits fully inside the tile.
    out.append(f'  <path d="{squircle(r - 4)}" fill="none" stroke="url(#edge)" stroke-width="8"/>')
    if small == 0:
        # The mark's soft shadow on the tile.
        for k in range(1, 9):
            out.append(f'  <g transform="translate(0 {k * 1.6:.1f})" opacity="0.05">{mark(INK)}</g>')
        out.append(f"  {mark('#FFFFFF')}")
    else:
        # Tiny sizes: bigger and bolder, so it holds at 16 px.
        zoom = [1.0, 1.14, 1.2][small]
        t = 512 - 512 * zoom
        out.append(f'  <g transform="translate({t:.1f} {t:.1f}) scale({zoom})">{mark("#FFFFFF", [0, 58, 66][small])}</g>')
    body = "\n".join(out)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>Opentern</title>
  <defs>
    <linearGradient id="fill" x1="164" y1="100" x2="860" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#7C8CFF"/>
      <stop offset="0.55" stop-color="#4F5BE0"/>
      <stop offset="1" stop-color="#3236A8"/>
    </linearGradient>
    <radialGradient id="glow" cx="350" cy="180" r="700" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.30"/>
      <stop offset="0.65" stop-color="#FFFFFF" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="edge" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.45"/>
      <stop offset="0.3" stop-color="#FFFFFF" stop-opacity="0"/>
      <stop offset="0.75" stop-color="{INK}" stop-opacity="0"/>
      <stop offset="1" stop-color="{INK}" stop-opacity="0.30"/>
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
