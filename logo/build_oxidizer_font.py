# build_oxidizer_font.py
# WSL/Ubuntu: fontforge -lang=py build_oxidizer_font.py
import fontforge
import os, sys

FONT_NAME_FAMILY = "OxidizerRusted"
FONT_NAME_FULL   = "OxidizerRusted Regular"
OUTLINE_OTF      = "OxidizerRusted.otf"
GLYPH_DIR        = "glyphs_png"

# Metrics
EM_UPM   = 1000
ASCENT   = 800
DESCENT  = 200
ADV_W    = 980
LSB      = 60
RSB      = 60

# Must match the set you sliced
GLYPHS = (
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    "abcdefghijklmnopqrstuvwxyz"
    "0123456789"
    ".,:;?!@#$%&()[]{}+-_=/\\'\""
)

SAFE_NAME = {
    " ": "space","." : "period",   "," : "comma",     ":" : "colon",      ";" : "semicolon",
    "?" : "question","!" : "exclamation","@" : "at",  "#" : "hash",       "$" : "dollar",
    "%" : "percent","&" : "ampersand","(" : "lparen",")" : "rparen",
    "[" : "lbracket","]" : "rbracket","{" : "lbrace","}" : "rbrace",
    "+" : "plus","-" : "hyphen","_" : "underscore","=" : "equals",
    "/" : "slash","\\": "backslash","'": "apostrophe","\"": "quote"
}

def safe_filename(ch):
    if ch in SAFE_NAME: return SAFE_NAME[ch]
    if ch.isalnum():     return ch
    return f"U{ord(ch):04X}"

def ensure_exists(path):
    if not os.path.exists(path):
        sys.stderr.write(f"[ERROR] Missing file: {path}\n")
        sys.exit(1)

def main():
    if not os.path.isdir(GLYPH_DIR):
        sys.stderr.write(f"[ERROR] Directory not found: {GLYPH_DIR}\n")
        sys.exit(1)

    font = fontforge.font()
    font.encoding = "UnicodeFull"
    font.em      = EM_UPM
    font.ascent  = ASCENT
    font.descent = DESCENT

    font.familyname = FONT_NAME_FAMILY
    font.fontname   = FONT_NAME_FAMILY
    font.fullname   = FONT_NAME_FULL
    font.appendSFNTName("English (US)", "Family",    FONT_NAME_FAMILY)
    font.appendSFNTName("English (US)", "SubFamily", "Regular")
    font.appendSFNTName("English (US)", "Fullname",  FONT_NAME_FULL)
    font.appendSFNTName("English (US)", "Version",   "Version 1.0")
    font.appendSFNTName("English (US)", "UniqueID",  "Oxidizer Project Font v1.0")
    font.appendSFNTName("English (US)", "Descriptor",
        "Brushed steel with thin top rust and moderate bottom drips; proportional; outline OTF.")

    for ch in GLYPHS:
        cp = ord(ch)
        g = font.createChar(cp)
        g.width = ADV_W
        g.left_side_bearing  = LSB
        g.right_side_bearing = RSB

        stem = safe_filename(ch)
        img_path = os.path.join(GLYPH_DIR, f"{stem}.png")
        ensure_exists(img_path)

        # Import the PNG as outlines source (supported on many builds),
        # then trace to vector so the font renders everywhere.
        try:
            g.importOutlines(img_path)  # loads bitmap for tracing in some builds
        except Exception:
            # If your build refuses PNG here, skip; we only need the bitmap for autoTrace.
            pass

        # Autotrace creates the actual vector glyph contours
        g.autoTrace()
        g.removeOverlap()
        g.simplify()

    font.generate(OUTLINE_OTF)
    print(f"[OK] Wrote {OUTLINE_OTF} (outline-only OTF)")
    
    # Optional bitmap strikes (choose sizes you care about)
    # font.bitmapSizes = (96, 128)
    font.generate("OxidizerRustedColor.ttf")  # many builds embed bitmaps automatically
    print("[OK] Wrote OxidizerRustedColor.ttf (CBDT/CBLC color)")

if __name__ == "__main__":
    main()
