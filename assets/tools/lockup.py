#!/usr/bin/env python3
"""Build the lt lockup (the Ligus mark, the wordmark "lt" and "Ligus terminal")
as SVGs with the text outlined, in light and dark variants, and the source
of the GitHub social preview (the light lockup, the tagline and a fragment
of the workflow-run screenshot).

The mark and its colours are the Ligus brand kit's (ligus-mark.svg,
ligus-brand-tokens.json); the wordmark follows the kit's horizontal lockup
(LigusLogo.html): Inter bold, letter-spacing -0.04em, the mark 52/42 of the
font size, a 12/42 gap. Needs fontTools and Inter (SIL OFL), which is not
vendored here: https://github.com/rsms/inter/releases

    python3 assets/tools/lockup.py path/to/Inter-Bold.ttf path/to/Inter-Medium.ttf
    rsvg-convert assets/social-preview.svg -o assets/social-preview.png
"""

import sys
from pathlib import Path

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

OUT = Path(__file__).resolve().parent.parent

# ligus-brand-tokens.json (studio)
INK = "#221f1a"
SOFT = "#8a8073"
PAPER = "#f4f1ea"
# A lighter soft for the dark variant, between soft and paper.
SOFT_ON_DARK = "#b3a999"

# ligus-mark.svg, verbatim: the lit core, right and up-left cells take the
# warm gradient; the ring alternates the two creams.
MARK_DEFS = (
    '<linearGradient id="{id}" x1="0" y1="0" x2=".4" y2="1">'
    '<stop offset="0" stop-color="#f0b878"/><stop offset="1" stop-color="#d98a44"/></linearGradient>'
)
MARK_CELLS = [
    ("M0 -12.6 10.91 -6.3V6.3L0 12.6-10.91 6.3V-6.3Z", "warm"),
    ("M24.25 -12.6 35.16 -6.3V6.3L24.25 12.6 13.34 6.3V-6.3Z", "warm"),
    ("M12.12 8.4 23.04 14.7V27.3L12.12 33.6 1.21 27.3V14.7Z", "#f0e0b8"),
    ("M-12.12 8.4-1.21 14.7V27.3L-12.12 33.6-23.04 27.3V14.7Z", "#fbf1d8"),
    ("M-24.25 -12.6-13.34 -6.3V6.3L-24.25 12.6-35.16 6.3V-6.3Z", "#f0e0b8"),
    ("M-12.12 -33.6-1.21 -27.3V-14.7L-12.12 -8.4-23.04 -14.7V-27.3Z", "warm"),
    ("M12.12 -33.6 23.04 -27.3V-14.7L12.12 -8.4 1.21 -14.7V-27.3Z", "#f0e0b8"),
]


def outline(font, text, size, tracking_em, x0, baseline):
    """Outlined text as one path's d, and its advance width."""
    upem = font["head"].unitsPerEm
    gs = font.getGlyphSet()
    cmap = font.getBestCmap()
    scale = size / upem
    pen = SVGPathPen(gs, ntos=lambda v: f"{v:.2f}".rstrip("0").rstrip("."))
    x = x0
    for i, ch in enumerate(text):
        g = gs[cmap[ord(ch)]]
        g.draw(TransformPen(pen, (scale, 0, 0, -scale, x, baseline)))
        x += g.width * scale + (tracking_em * size if i < len(text) - 1 else 0)
    return pen.getCommands(), x - x0


def ink_left(font, ch, size):
    gs = font.getGlyphSet()
    b = BoundsPen(gs)
    gs[font.getBestCmap()[ord(ch)]].draw(b)
    return b.bounds[0] * size / font["head"].unitsPerEm


def build(bold, medium, dark):
    fs = 88  # wordmark size
    mark = fs * 52 / 42
    gap = fs * 12 / 42
    pad = 8
    sub = 30  # "Ligus terminal"
    height = 100
    baseline = 82
    cap = bold["OS/2"].sCapHeight / bold["head"].unitsPerEm * fs
    cy = baseline - cap / 2
    mx = pad + mark / 2
    wx = pad + mark + gap - ink_left(bold, "l", fs)
    word, ww = outline(bold, "lt", fs, -0.04, wx, baseline)
    sx = wx + ww + fs * 0.26
    subtitle, sw = outline(medium, "Ligus terminal", sub, -0.01, sx, baseline)
    width = round(sx + sw + pad)
    gid = "lt-warm-dark" if dark else "lt-warm"
    cells = "".join(
        f'<path d="{d}" fill="{("url(#" + gid + ")") if fill == "warm" else fill}"/>' for d, fill in MARK_CELLS
    )
    ink, soft = (PAPER, SOFT_ON_DARK) if dark else (INK, SOFT)
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" '
        f'role="img" aria-label="lt, the Ligus terminal"><title>lt, the Ligus terminal</title>'
        f"<defs>{MARK_DEFS.format(id=gid)}</defs>"
        f'<g transform="translate({mx:.2f} {cy:.2f}) scale({mark / 92:.4f})">{cells}</g>'
        f'<path fill="{ink}" d="{word}"/><path fill="{soft}" d="{subtitle}"/></svg>\n'
    )


# The README's first line, as the preview's tagline.
TAGLINE = "A terminal client for orchestrating Ligus agents."


def social(bold, medium, lockup):
    """1280x640: the lockup and tagline on the brand's paper, and the top-left
    of a real screen (screens/workflow-run.svg, rendered by
    examples/screenshots.rs) running off the bottom-right corner."""
    inner = lockup[lockup.index(">", lockup.index("</title>")) + 1 : lockup.rindex("</svg>")]
    inner = inner.replace('id="lt-warm"', 'id="sp-warm"').replace("url(#lt-warm)", "url(#sp-warm)")
    tag, _ = outline(medium, TAGLINE, 34, -0.01, 80, 292)
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" '
        'width="1280" height="640" viewBox="0 0 1280 640">'
        f'<rect width="1280" height="640" fill="{PAPER}"/>'
        f'<g transform="translate(64 80) scale(1.45)">{inner}</g>'
        f'<path fill="{INK}" d="{tag}"/>'
        '<rect x="384" y="368" width="960" height="560" rx="12" fill="#221f1a" opacity=".18" transform="translate(0 6)"/>'
        '<image x="380" y="360" width="901.6" height="526" href="screens/workflow-run.svg" '
        'xlink:href="screens/workflow-run.svg"/></svg>\n'
    )


def main():
    bold, medium = TTFont(sys.argv[1]), TTFont(sys.argv[2])
    for dark in (False, True):
        name = "lt-lockup-dark.svg" if dark else "lt-lockup-light.svg"
        (OUT / name).write_text(build(bold, medium, dark))
        print(OUT / name)
    (OUT / "social-preview.svg").write_text(social(bold, medium, build(bold, medium, False)))
    print(OUT / "social-preview.svg")


if __name__ == "__main__":
    main()
