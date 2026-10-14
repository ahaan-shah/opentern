#!/usr/bin/env python3
"""Regenerates the Opentern icon.

A tern gliding up and to the right over a faint rising price line, on a
sea-blue tile. The tile is the same true superellipse (n = 5) as Magpie's,
so the two apps look like siblings. The tern has the bird's two giveaways:
a dark cap and an orange beak.

packaging/linux/opentern.svg is the icon everywhere on Linux.
opentern-24.svg and opentern-16.svg are simpler variants for tiny sizes (no
price line, no shadow, a bigger bird). Two rasters are made with resvg (or
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

# The tern, on the 1024 grid: body and wing in one white shape, the beak
# and the cap on top of it.
BODY = (
    "M742 432C738 402 716 380 690 380C668 380 650 390 638 404"
    "C590 362 500 282 316 204C402 290 472 380 502 452"
    "C470 478 432 506 394 530L186 500C254 538 296 558 328 574"
    "L224 666C300 632 362 612 422 598C522 580 612 540 670 500"
    "C706 476 732 456 742 432Z"
)
BEAK = "M732 410L842 402L736 440Z"
CAP = "M742 426C738 398 716 380 690 380C668 380 650 390 638 404C664 404 696 412 718 422C730 428 738 432 742 426Z"
# The price line the tern rides above.
LINE = "M206 790L330 722L428 762L560 652L660 690L826 566"


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


def tern(extra=""):
    return (
        f'    <path d="{BODY}" fill="#FFFFFF"{extra}/>\n'
        f'    <path d="{BEAK}" fill="#FFA34D"/>\n'
        f'    <path d="{CAP}" fill="#16233F"/>'
    )


def svg(small=0):
    """small=0 is the master; 1 and 2 are the 24px and 16px variants."""
    r = [412.0, 470.0, 492.0][small]
    tile = squircle(r)
    out = []
    if small == 0:
        # Drop shadow: 14 faint, progressively larger and lower copies.
        for k in range(14, 0, -1):
            grow = k * 2.0
            out.append(
                f'  <path transform="translate(0 {6 + k * 0.9:.1f})" d="{squircle(r + grow, 48)}" '
                f'fill="#06163A" fill-opacity="0.024"/>'
            )
    out.append(f'  <path d="{tile}" fill="url(#fill)"/>')
    out.append(f'  <path d="{tile}" fill="url(#glow)"/>')
    # Rim light: a stroke on an inset outline, so it sits fully inside the tile.
    out.append(f'  <path d="{squircle(r - 4)}" fill="none" stroke="url(#edge)" stroke-width="8"/>')
    if small == 0:
        out.append(
            f'  <path d="{LINE}" fill="none" stroke="#FFFFFF" stroke-opacity="0.26" '
            'stroke-width="22" stroke-linecap="round" stroke-linejoin="round"/>'
        )
        out.append('  <g transform="translate(0 40)">')
        # The bird's soft shadow on the tile.
        for k in range(1, 9):
            out.append(
                f'    <path transform="translate(0 {k * 1.6:.1f})" d="{BODY}" fill="#0A2556" fill-opacity="0.05"/>'
            )
        out.append(tern())
    else:
        # Tiny sizes: the bird alone, bigger and centred, edged so it holds.
        zoom = [1.0, 1.18, 1.26][small]
        cx, cy = 514.0, 435.0
        tx, ty = 512 - cx * zoom, 512 - cy * zoom
        out.append(f'  <g transform="translate({tx:.1f} {ty:.1f}) scale({zoom})">')
        weight = ["", ' stroke="#FFFFFF" stroke-width="10" stroke-linejoin="round"',
                  ' stroke="#FFFFFF" stroke-width="18" stroke-linejoin="round"'][small]
        out.append(tern(weight))
    out.append("  </g>")
    body = "\n".join(out)
    return f"""<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>Opentern</title>
  <defs>
    <linearGradient id="fill" x1="164" y1="100" x2="860" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#78B4FA"/>
      <stop offset="0.55" stop-color="#3D7FDF"/>
      <stop offset="1" stop-color="#1F4FA6"/>
    </linearGradient>
    <radialGradient id="glow" cx="350" cy="180" r="700" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.30"/>
      <stop offset="0.65" stop-color="#FFFFFF" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="edge" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.45"/>
      <stop offset="0.3" stop-color="#FFFFFF" stop-opacity="0"/>
      <stop offset="0.75" stop-color="#0A2556" stop-opacity="0"/>
      <stop offset="1" stop-color="#0A2556" stop-opacity="0.30"/>
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
