"""Draws Opentern's commodity icons (64x64 squares; the app crops them round)."""
import os, sys

OUT = sys.argv[1] if len(sys.argv) > 1 else "crates/app/assets/commodities"
os.makedirs(OUT, exist_ok=True)


def bg(a, b):
    return (f'<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">'
            f'<stop offset="0" stop-color="{a}"/><stop offset="1" stop-color="{b}"/>'
            f'</linearGradient></defs><rect width="64" height="64" fill="url(#bg)"/>')


def svg(body):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">{body}</svg>\n'


def bar(x, y, face, top, edge, w=18, h=8, inset=3):
    """An ingot seen from the front: a trapezoid with a lit top."""
    return (f'<path d="M{x} {y+h}H{x+w}L{x+w-inset} {y}H{x+inset}Z" fill="{face}" '
            f'stroke="{edge}" stroke-width="1" stroke-linejoin="round"/>'
            f'<path d="M{x+inset+0.6} {y+0.6}H{x+w-inset-0.6}L{x+w-inset-0.2} {y+2.4}H{x+inset+0.2}Z" fill="{top}"/>')


def stack(face, top, edge):
    k = dict(w=22, h=10, inset=3.5)
    return (bar(9.5, 37, face, top, edge, **k) + bar(32.5, 37, face, top, edge, **k)
            + bar(21, 25.5, face, top, edge, **k))


GLYPHS = {  # stroked letters in a 10-high box: (width, path)
    "P": (5.5, "M0 10V0H3a3 3 0 0 1 0 6H0"),
    "C": (6, "M6 1.6A4.3 5 0 1 0 6 8.4"),
    "t": (4.5, "M1.6 1V8.3a1.6 1.6 0 0 0 1.6 1.7H4.5M0 3.6H4.2"),
    "d": (6.5, "M6.5 0V10M6.5 6.8a3.1 3.2 0 1 1 0-.2"),
    "u": (5.5, "M0 3.6V7.4a2.6 2.6 0 0 0 5.2 0V3.6M5.2 3.6V10"),
}


def word(letters, cx, top, ink, gap=1.6, sw=2.1):
    total = sum(GLYPHS[c][0] for c in letters) + gap * (len(letters) - 1)
    x, out = cx - total / 2, []
    for c in letters:
        w, d = GLYPHS[c]
        out.append(f'<path d="{d}" transform="translate({x:.2f} {top})" fill="none" stroke="{ink}" '
                   f'stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round"/>')
        x += w + gap
    return "".join(out)


def ingot(face, top, edge, letters, ink):
    x, y, w, h = 11, 22, 42, 20
    return (f'<path d="M{x} {y+h}H{x+w}L{x+w-6} {y}H{x+6}Z" fill="{face}" stroke="{edge}" '
            f'stroke-width="1.2" stroke-linejoin="round"/>'
            f'<path d="M{x+7} {y+1}H{x+w-7}L{x+w-6.2} {y+3.6}H{x+6.2}Z" fill="{top}"/>'
            + word(letters, 32, y + 6.5, ink))


def barrel(body, band, cap):
    return (f'<rect x="21" y="17" width="22" height="31" rx="4" fill="{body}"/>'
            f'<rect x="21" y="24" width="22" height="2.6" fill="{band}"/>'
            f'<rect x="21" y="38.4" width="22" height="2.6" fill="{band}"/>'
            f'<ellipse cx="32" cy="18.2" rx="10" ry="2.2" fill="{cap}"/>'
            f'<path d="M32 28.6c-1.7 2.4-2.6 3.9-2.6 5.1a2.6 2.6 0 0 0 5.2 0c0-1.2-.9-2.7-2.6-5.1z" fill="{band}"/>')


icons = {}

icons["gold"] = svg(bg("#E9B23A", "#A8740C") + stack("#FFE07A", "#FFF6CC", "#7E5300"))
icons["silver"] = svg(bg("#BFC7CF", "#7D8894") + stack("#EEF1F4", "#FFFFFF", "#5C6672"))
icons["platinum"] = svg(bg("#9FB0C0", "#5E7185") + ingot("#E6ECF1", "#FFFFFF", "#4E6074", "Pt", "#4E6074"))
icons["palladium"] = svg(bg("#A89C8E", "#6E6356") + ingot("#ECE7E0", "#FFFFFF", "#5C5246", "Pd", "#5C5246"))
icons["copper"] = svg(bg("#E0946A", "#9C4E26") + ingot("#F6C4A1", "#FFE3CF", "#7A3A18", "Cu", "#7A3A18"))

icons["wti"] = svg(bg("#3E4650", "#1C2128") + barrel("#F2F4F6", "#2A3038", "#C9CED4"))
icons["brent"] = svg(bg("#2C5282", "#152B47") + barrel("#F2F4F6", "#1E3A5F", "#C9D3DE"))

icons["natgas"] = svg(bg("#24324A", "#0F1726") +
    '<defs><linearGradient id="fl" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#8FD3FF"/>'
    '<stop offset="1" stop-color="#2F6BFF"/></linearGradient></defs>'
    '<path d="M32 14c1.5 6.5 9.5 10.5 9.5 19.5a9.5 9.5 0 0 1-19 0c0-4.2 2-7.2 4-9 .2 3 1.4 5 3.2 5.8-.8-5.8.4-11 2.3-16.3z" fill="url(#fl)"/>'
    '<path d="M32 33c.7 2.7 4 4.3 4 7.6a4 4 0 0 1-8 0c0-2.4 1.6-4.5 4-7.6z" fill="#E8F6FF"/>')

icons["gasoline"] = svg(bg("#F07A3A", "#B8410E") +
    '<rect x="19" y="16" width="18" height="32" rx="2.5" fill="#FFFFFF"/>'
    '<rect x="22" y="20" width="12" height="8" rx="1.2" fill="#C9541C"/>'
    '<rect x="16.5" y="46" width="23" height="3" rx="1.5" fill="#FFFFFF"/>'
    '<path d="M37 22h3.2a2.8 2.8 0 0 1 2.8 2.8V39a2 2 0 0 0 4 0V27l-3.5-3.5" fill="none" stroke="#FFFFFF" '
    'stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>')

icons["heating-oil"] = svg(bg("#9A5B24", "#5A3210") +
    '<path d="M32 15c-5 7.2-8.5 12-8.5 16.3a8.5 8.5 0 0 0 17 0C40.5 27 37 22.2 32 15z" fill="#FFD9A8"/>'
    '<path d="M27.5 31.5a4.6 4.6 0 0 0 4.5 4.6" fill="none" stroke="#FFF4E6" stroke-width="1.8" stroke-linecap="round"/>'
    + "".join(f'<path d="M{x} 50c-1.6-1.6 1.6-3.2 0-4.8s1.6-3.2 0-4.8" fill="none" stroke="#FFB061" '
              f'stroke-width="2" stroke-linecap="round"/>' for x in (25, 32, 39)))


def grains(cx, top, n, rx, ry, colour, spread):
    out = []
    for i in range(n):
        y = top + i * (ry * 1.55)
        for side in (-1, 1):
            x = cx + side * spread
            ang = side * 28
            out.append(f'<ellipse cx="{x:.1f}" cy="{y:.1f}" rx="{rx}" ry="{ry}" fill="{colour}" '
                       f'transform="rotate({ang} {x:.1f} {y:.1f})"/>')
    out.append(f'<ellipse cx="{cx}" cy="{top - ry*1.3:.1f}" rx="{rx}" ry="{ry}" fill="{colour}"/>')
    return "".join(out)


icons["wheat"] = svg(bg("#D9A441", "#9E6B17") +
    '<path d="M32 22V51" stroke="#FFF3D1" stroke-width="2" stroke-linecap="round"/>' +
    grains(32, 22, 5, 2.2, 3.8, "#FFF3D1", 3.4) +
    '<path d="M32 46c-3-1-6-3.5-7.5-7M32 46c3-1 6-3.5 7.5-7" fill="none" stroke="#FFF3D1" stroke-width="1.6" stroke-linecap="round"/>')

kernels = "".join(
    f'<circle cx="{28.2 + c*2.6 + (r % 2)*1.3:.1f}" cy="{19 + r*2.7:.1f}" r="1.05" fill="#E0A100"/>'
    for r in range(9) for c in range(3 if r % 2 else 4) if not (r in (0, 8) and c in (0, 3)))
icons["corn"] = svg(bg("#5FA343", "#2F6B22") +
    '<ellipse cx="32" cy="30" rx="6.6" ry="14" fill="#FFD84A"/>' + kernels +
    '<path d="M32 50c-6-2-9.5-8-9.5-17 2.8 3 5.2 7 6.8 11.5L32 50z" fill="#CDEFB0"/>'
    '<path d="M32 50c6-2 9.5-8 9.5-17-2.8 3-5.2 7-6.8 11.5L32 50z" fill="#E7F8D8"/>')

icons["soybeans"] = svg(bg("#7DA33A", "#476A18") +
    '<path d="M17 38c2-9 12-17 27-20 3.2-.6 4.6 2.6 3 5-4.5 9-14.5 17.5-25.5 19.5-3.3.6-5.2-1.4-4.5-4.5z" fill="#D9EBB0"/>'
    + "".join(f'<circle cx="{x}" cy="{y}" r="4.3" fill="#F6E9B8" stroke="#B8A25E" stroke-width="0.8"/>'
              for x, y in ((24, 36), (31.5, 30.5), (39, 25))))

icons["coffee"] = svg(bg("#8A5A3B", "#4A2C18") +
    '<ellipse cx="32" cy="32" rx="11" ry="15" transform="rotate(30 32 32)" fill="#F2DCC6"/>'
    '<path d="M38.5 19.5c-6 3.5-2.5 9.5-6.5 13s-8.5 4-6.5 12" fill="none" stroke="#6B4027" stroke-width="2.4" stroke-linecap="round"/>')


def cube(x, y, s, top, left, right):
    h = s * 0.5
    return (f'<path d="M{x} {y}L{x+s} {y-h}L{x+2*s} {y}L{x+s} {y+h}Z" fill="{top}"/>'
            f'<path d="M{x} {y}L{x+s} {y+h}V{y+h+s*1.1}L{x} {y+s*1.1}Z" fill="{left}"/>'
            f'<path d="M{x+2*s} {y}L{x+s} {y+h}V{y+h+s*1.1}L{x+2*s} {y+s*1.1}Z" fill="{right}"/>')


icons["sugar"] = svg(bg("#7DB8E8", "#3E7FC1") +
    cube(17, 36, 7.5, "#FFFFFF", "#DCE8F4", "#BCD2E8") + cube(32, 36, 7.5, "#FFFFFF", "#DCE8F4", "#BCD2E8")
    + cube(24.5, 25, 7.5, "#FFFFFF", "#DCE8F4", "#BCD2E8"))

icons["cocoa"] = svg(bg("#7A4A2C", "#3E2314") +
    '<g transform="rotate(-35 32 32)">'
    '<ellipse cx="32" cy="32" rx="9.5" ry="16" fill="#F0A04B"/>'
    '<path d="M32 16.5V47.5M27 18.5c-2 8-2 19 0 27M37 18.5c2 8 2 19 0 27" fill="none" stroke="#C26A1E" stroke-width="1.5"/>'
    '<path d="M32 16c0-2 .6-3.4 2-4.4" stroke="#7FB04A" stroke-width="2.2" fill="none" stroke-linecap="round"/>'
    '</g>')

icons["cotton"] = svg(bg("#6F8FAE", "#3C5A78") +
    '<path d="M24 46l8-6 8 6M32 40v9" fill="none" stroke="#8B5E34" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>'
    + "".join(f'<circle cx="{x}" cy="{y}" r="{r}" fill="#FFFFFF"/>'
              for x, y, r in ((25, 32, 7.5), (39, 32, 7.5), (32, 25, 8), (32, 35, 7.5)))
    + '<path d="M25 32c3 1 4.5 2.5 7 3M39 32c-3 1-4.5 2.5-7 3M32 25v7" fill="none" stroke="#DCE5EE" stroke-width="1.2" stroke-linecap="round"/>')

icons["cattle"] = svg(bg("#A0704A", "#5E3A1F") +
    # horns, ears, head, muzzle, nostrils
    '<path d="M22 21c-4-1-6-4-6-7 3 2 6 2.5 9 2.5M42 21c4-1 6-4 6-7-3 2-6 2.5-9 2.5" fill="#F4E9D8"/>'
    '<ellipse cx="17.5" cy="26" rx="5" ry="2.8" transform="rotate(20 17.5 26)" fill="#FFFFFF"/>'
    '<ellipse cx="46.5" cy="26" rx="5" ry="2.8" transform="rotate(-20 46.5 26)" fill="#FFFFFF"/>'
    '<path d="M22 22c0-3 4-5 10-5s10 2 10 5l-1.5 14h-17z" fill="#FFFFFF"/>'
    '<rect x="22.5" y="33" width="19" height="14" rx="7" fill="#F3C9B5"/>'
    '<ellipse cx="28" cy="40" rx="1.6" ry="2.1" fill="#7A4A30"/><ellipse cx="36" cy="40" rx="1.6" ry="2.1" fill="#7A4A30"/>'
    '<circle cx="27" cy="27" r="1.7" fill="#3A2416"/><circle cx="37" cy="27" r="1.7" fill="#3A2416"/>')

for name, s in icons.items():
    with open(os.path.join(OUT, f"{name}.svg"), "w") as f:
        f.write(s)
print(len(icons), "icons")
