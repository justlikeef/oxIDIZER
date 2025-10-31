# build_oxidizer_otf_svg.py
# Build a color OpenType (OTF, SVG table) using FontForge's Python API
# Run with:
#   fontforge -lang=py build_oxidizer_otf_svg.py
# or on Windows builds with ffpython.exe

import fontforge
import psMat
import os
import sys

FONT_NAME_FAMILY = "OxidizerRusted"
FONT_NAME_FULL   = "OxidizerRusted Regular"
OUTPUT_OTF       = "OxidizerRusted.otf"
GLYPH_DIR        = "glyphs_png"

# Proportional metrics
DEFAULT_WIDTH = 980
LB = 60
RB = 60

# Order & coverage (must match the set you sliced)
GLYPHS = (
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    "abcdefghijklmnopqrstuvwxyz"
    "0123456789"
    ".,:;?!@#$%&()[]{}+-_=/\\'\""
)

# Mapping of characters -> safe filenames (no extension)
SAFE_NAME = {
    " ": "space",
    ".": "period",
    ",": "comma",
    ":": "colon",
    ";": "semicolon",
    "?": "question",
    "!": "exclamation",
    "@": "at",
    "#": "hash",
    "$": "dollar",
    "%": "percent",
    "&": "ampersand",
    "(": "lparen",
    ")": "rparen",
    "[": "lbracket",
    "]": "rbracket",
    "{": "lbrace",
    "}": "rbrace",
    "+": "plus",
    "-": "hyphen",
    "_": "underscore",
    "=": "equals",
    "/": "slash",
    "\\": "backslash",
    "'": "apostrophe",
    "\"": "quote",
}

def safe_filename(ch: str) -> str:
    if ch in SAFE_NAME:
        return SAFE_NAME[ch]
    if ch.isalnum():
        return ch
    return f"U{ord(ch):04X}"

def ensure_exists(path):
    if not os.path.exists(path):
        sys.stderr.write(f"[ERROR] Missing file: {path}\n")
        sys.exit(1)

def main():
    # sanity checks
    if not os.path.isdir(GLYPH_DIR):
        sys.stderr.write(f"[ERROR] Directory not found: {GLYPH_DIR}\n")
        sys.exit(1)

    # Create new font
    font = fontforge.font()
    font.encoding = "UnicodeFull"
    font.em = 1000

    # Name table
    font.familyname  = FONT_NAME_FAMILY
    font.fontname    = FONT_NAME_FAMILY
    font.fullname    = FONT_NAME_FULL
    font.appendSFNTName("English (US)", "Family", FONT_NAME_FAMILY)
    font.appendSFNTName("English (US)", "SubFamily", "Regular")
    font.appendSFNTName("English (US)", "Fullname", FONT_NAME_FULL)
    font.appendSFNTName("English (US)", "Version", "Version 1.0")
    font.appendSFNTName("English (US)", "UniqueID", "Oxidizer Project Font v1.0")
    font.appendSFNTName("English (US)", "Descriptor",
                        "Brushed steel with thin top rust and moderate bottom drips; proportional spacing; SVG-in-OTF color.")

    # Build glyphs
    for ch in GLYPHS:
        codepoint = ord(ch)
        g = font.createChar(codepoint)
        g.width = DEFAULT_WIDTH
        g.left_side_bearing  = LB
        g.right_side_bearing = RB

        stem = safe_filename(ch)
        img_path = os.path.join(GLYPH_DIR, f"{stem}.png")
        ensure_exists(img_path)

        # Import the color PNG. For color OTF (SVG table), FontForge will wrap images as per-glyph SVGs during generate().
        # Some FontForge builds prefer importImage over importOutlines for bitmaps.
        try:
            g.importImage(img_path)
        except Exception:
            # Fallback—some builds accept importOutlines for images.
            g.importOutlines(img_path)

        # Optional: scale/transform if your images are too big/small
        # For example, to scale 90% about origin:
        # mat = psMat.scale(0.9)
        # g.transform(mat)

        # Keep glyph on baseline; if images are offset, you can translate:
        # g.transform(psMat.translate(0, 0))

    # Generate OTF with SVG color table.
    # Flags:
    #   'opentype' -> OTF generation
    #   'svg'      -> include SVG table for color glyphs
    # Many builds accept the flags tuple; others infer from file ext + embedded images.
    try:
        font.generate(OUTPUT_OTF, flags=("opentype", "svg"))
    except Exception:
        # Fallback for older builds
        font.generate(OUTPUT_OTF)

    print(f"[OK] Wrote {OUTPUT_OTF}")

if __name__ == "__main__":
    main()
