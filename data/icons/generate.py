"""Generate AcerUltraX icons (SVG) from one X geometry traced from the user's logo (1024 px canvas)."""
import os, sys

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)

# The X, traced from the logo: four blades with a bevel kink on their outer (top/bottom) edges.
TL, a, Tn, b, TR = (283, 285), (420, 338), (512, 457), (604, 338), (741, 285)
Rn, BR, c, Bn, d = (567, 512), (741, 739), (604, 686), (512, 567), (420, 686)
BL, Ln, C = (283, 739), (457, 512), (512, 512)
OUTLINE = [TL, a, Tn, b, TR, Rn, BR, c, Bn, d, BL, Ln]
# (points, fill) per facet; light comes from the top left.
FACETS = [
    ([TL, a, Tn, C], "url(#f1)"),
    ([TL, C, Ln], "#2a2d32"),
    ([Tn, b, TR, C], "url(#f2)"),
    ([TR, Rn, C], "#1c1f23"),
    ([BL, Ln, C], "#3b3f45"),
    ([BL, C, Bn, d], "#16181b"),
    ([BR, Rn, C], "#33373c"),
    ([BR, c, Bn, C], "#121417"),
]
CYAN = "#22d3ee"


def pts(p):
    return " ".join(f"{x},{y}" for x, y in p)


def x_mark(stroke=7.0, glow=True):
    """The faceted X with its cyan edge (in 1024 canvas coordinates)."""
    s = []
    if glow:
        s.append('<circle cx="512" cy="512" r="230" fill="url(#glow)"/>')
        s.append(f'<polygon points="{pts(OUTLINE)}" fill="none" stroke="{CYAN}" stroke-width="{stroke * 3}" '
                 'stroke-linejoin="miter" opacity=".55" filter="url(#blur)"/>')
    for p, f in FACETS:
        s.append(f'<polygon points="{pts(p)}" fill="{f}"/>')
    # Ridge highlights along the blades.
    s.append(f'<polyline points="{pts([TL, C, BR])}" stroke="#ffffff" stroke-opacity=".10" stroke-width="2" fill="none"/>')
    s.append(f'<polyline points="{pts([TR, C, BL])}" stroke="#ffffff" stroke-opacity=".07" stroke-width="2" fill="none"/>')
    s.append(f'<polygon points="{pts(OUTLINE)}" fill="none" stroke="{CYAN}" stroke-width="{stroke}" '
             'stroke-linejoin="miter" stroke-miterlimit="12"/>')
    return "\n  ".join(s)


DEFS = f"""<defs>
  <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#2c3037"/><stop offset=".55" stop-color="#121519"/><stop offset="1" stop-color="#07090c"/>
  </linearGradient>
  <linearGradient id="rim" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#5a616b"/><stop offset=".5" stop-color="#262b32"/><stop offset="1" stop-color="#1a1e23"/>
  </linearGradient>
  <linearGradient id="f1" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#7a7f86"/><stop offset="1" stop-color="#4a4e54"/>
  </linearGradient>
  <linearGradient id="f2" x1="1" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#5d6268"/><stop offset="1" stop-color="#3c4046"/>
  </linearGradient>
  <radialGradient id="glow">
    <stop offset="0" stop-color="{CYAN}" stop-opacity=".95"/><stop offset=".4" stop-color="{CYAN}" stop-opacity=".45"/>
    <stop offset="1" stop-color="{CYAN}" stop-opacity="0"/>
  </radialGradient>
  <clipPath id="tileclip"><rect x="64" y="64" width="896" height="896" rx="200"/></clipPath>
  <filter id="blur" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="10"/></filter>
</defs>"""


def svg(body, size=1024, view="0 0 1024 1024"):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="{view}">\n'
            f'{DEFS}\n  {body}\n</svg>\n')


def tile():
    """The dark rounded tile with the diagonal sheen of the logo."""
    return ('<rect x="64" y="64" width="896" height="896" rx="200" fill="url(#bg)"/>\n'
            '  <path d="M 960 64 L 960 300 L 300 960 L 64 960 L 64 760 L 760 64 Z" clip-path="url(#tileclip)" '
            'fill="#ffffff" opacity=".035"/>\n'
            '  <rect x="66" y="66" width="892" height="892" rx="198" fill="none" stroke="url(#rim)" stroke-width="4"/>')


def scaled(body, k):
    return f'<g transform="translate(512 512) scale({k}) translate(-512 -512)">\n  {body}\n  </g>'


# 1. App icon (launcher, window, about): tile + faceted X, X enlarged to fill the tile like the logo.
app = svg(tile() + "\n  " + scaled(x_mark(), 1.3))
open(f"{OUT}/app.svg", "w").write(app)

# 2. App icon for 16-32 px: same tile, solid cyan X (facets vanish at that size).
small = svg(tile() + "\n  " + scaled(f'<polygon points="{pts(OUTLINE)}" fill="{CYAN}"/>', 1.4))
open(f"{OUT}/app-small.svg", "w").write(small)

# 3. The mark alone (in-app brand, README): faceted X with glow, transparent, cropped.
open(f"{OUT}/mark.svg", "w").write(svg(x_mark(stroke=9), view="200 200 624 624"))


# 4. Top-bar (symbolic) icons, 16 px grid, one colour; the desktop recolours them to match the panel.
def symbolic(body):
    return ('<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">\n'
            f'  {body}\n</svg>\n')


def to16(p, k=1.0):
    # Map the X's bounding box (283..741) onto 1..15, scaled by k around the centre.
    return [(8 + (x - 512) / 458 * 14 * k, 8 + (y - 512) / 454 * 14 * k) for x, y in p]


def p16(p, k=1.0):
    return " ".join(f"{x:.2f},{y:.2f}" for x, y in to16(p, k))


FG = "#2e3436"  # GNOME's symbolic base colour
# A: solid X.
open(f"{OUT}/tray-solid.svg", "w").write(symbolic(f'<polygon points="{p16(OUTLINE)}" fill="{FG}"/>'))
# B: two-tone X (upper faces full, lower faces dimmed), keeps a hint of the 3D facets.
two = []
for i, (p, _) in enumerate(FACETS):
    lit = i in (0, 2, 4, 6)
    two.append(f'<polygon points="{p16(p)}" fill="{FG}"{"" if lit else " opacity=\".55\""} '
               f'stroke="{FG}" stroke-width=".15"/>')
open(f"{OUT}/tray-facets.svg", "w").write(symbolic("\n  ".join(two)))
# C: rounded badge with the X cut out.
badge = (f'<path fill="{FG}" fill-rule="evenodd" d="M 4 0.5 H 12 A 3.5 3.5 0 0 1 15.5 4 V 12 A 3.5 3.5 0 0 1 12 15.5 '
         f'H 4 A 3.5 3.5 0 0 1 0.5 12 V 4 A 3.5 3.5 0 0 1 4 0.5 Z M {p16(OUTLINE, .72).replace(" ", " L ")} Z"/>')
open(f"{OUT}/tray-badge.svg", "w").write(symbolic(badge))
print("ok")
