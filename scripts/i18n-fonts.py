#!/usr/bin/env python3
"""Builds the small fonts that draw the interface in Russian, Hindi,
Japanese and Chinese: crates/app/assets/fonts/i18n/.

The interface fonts are Latin only and many computers have no Chinese or
Japanese font, so each script gets a subset of an open font (Inter, Noto
Sans; SIL Open Font License) with what the catalogs use. Re-run after the
catalogs change (a test says when a character is missing):

    pip install fonttools
    python3 scripts/i18n-fonts.py
"""

import pathlib
import re
import sys
import tempfile
import urllib.request

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = pathlib.Path(__file__).resolve().parent.parent
LOCALES = ROOT / "crates/app/locales"
OUT = ROOT / "crates/app/assets/fonts/i18n"
BASE = "https://github.com/google/fonts/raw/main/ofl/"
SOURCES = {
    "cyrillic": ("inter/Inter%5Bopsz,wght%5D.ttf", "Inter"),
    "devanagari": ("notosansdevanagari/NotoSansDevanagari%5Bwdth,wght%5D.ttf", "Noto Sans Devanagari"),
    "japanese": ("notosansjp/NotoSansJP%5Bwght%5D.ttf", "Noto Sans JP"),
    "chinese": ("notosanssc/NotoSansSC%5Bwght%5D.ttf", "Noto Sans SC"),
}
# Language names in the picker, whatever the language.
NAMES = "日本語简体中文"
KANA = "".join(map(chr, [*range(0x3000, 0x3040), *range(0x3041, 0x30FF), *range(0xFF01, 0xFF5F)]))


def used(code):
    text = (LOCALES / f"{code}.po").read_text(encoding="utf-8")
    return {c for s in re.findall(r'^(?:msgstr )?"(.*)"$', text, re.M) for c in s if ord(c) > 0x24F}


def chars(name):
    if name == "cyrillic":
        return set(map(chr, range(0x400, 0x460))) | set("№")
    if name == "devanagari":
        return set(map(chr, range(0x900, 0x980))) | set("‌‍₹")
    return (used("ja") if name == "japanese" else used("zh-Hans")) | set(NAMES) | set(KANA)


def main():
    cache = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path(tempfile.mkdtemp())
    OUT.mkdir(parents=True, exist_ok=True)
    for name, (url, family) in SOURCES.items():
        src = cache / f"{name}.ttf"
        if not src.exists():
            urllib.request.urlretrieve(BASE + url, src)
        for weight, label in ((400, "Regular"), (560, "Medium")):
            font = TTFont(src)
            axes = {a.axisTag: a.defaultValue for a in font["fvar"].axes}
            axes["wght"] = weight
            if "opsz" in axes:
                axes["opsz"] = 14
            font = instancer.instantiateVariableFont(font, axes)
            opts = subset.Options()
            opts.layout_features = ["*"] if name == "devanagari" else ["kern", "mark", "mkmk", "ccmp", "locl"]
            opts.hinting = False
            opts.name_IDs = [0, 1, 2, 13, 14]
            opts.notdef_outline = True
            s = subset.Subsetter(opts)
            s.populate(unicodes=[ord(c) for c in chars(name)])
            s.subset(font)
            out = OUT / f"{name}-{label}.ttf"
            font.save(out)
            print(f"{out.name}: {len(font.getBestCmap())} characters, {out.stat().st_size // 1024} KB ({family})")


main()
