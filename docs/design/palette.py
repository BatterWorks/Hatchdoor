#!/usr/bin/env python3
"""Check the callout accent palette against the Forge system.

The callout accents in frontend/src/styles/base.css are built to three rules
that are easy to break by hand-editing a hex:

  1. One lightness per theme, so no kind's label outweighs another's.
  2. Chroma ranked by distance from the Forge hue wedge — the further a hue
     sits from the 35°..95° band the rest of the system lives in, the greyer it
     has to be, so it reads as a press ink on cream rather than a UI status
     colour.
  3. At least 4.5:1 as small text on its own 12%-tinted surface, and for the
     two accents that double as chrome badge tokens, on the badge surface too.

Run it after touching any accent:  python3 docs/design/palette.py
Exits non-zero if a rule is broken. No dependencies.
"""

import math
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
BASE_CSS = REPO / "frontend/src/styles/base.css"

# Forge's chromatic tokens all sit in this OKLCh hue band.
FORGE_WEDGE = (35.0, 95.0)
TINT_PCT = 12  # .callout's surface mix, see note-content.css
MIN_CONTRAST = 4.5
MAX_L_SPREAD = 0.02

# Chroma a hue may carry, by how far outside the Forge wedge it sits.
# 0.115 is the house maximum, held by --err-fg on --hot's own hue.
CHROMA_CEILING = ((35, 0.115), (60, 0.090), (90, 0.075), (360, 0.055))

ACCENTS = (
    "co-info",
    "co-abstract",
    "co-tip",
    "co-success",
    "co-question",
    "co-example",
    "warn-fg",
    "err-fg",
    "co-fail",
)

# Accents that also colour chrome, with the surfaces they sit on there.
BADGES = {
    "warn-fg": {"light": "#fff6e7", "dark": "#2a2014"},
    "err-fg": {"light": "#faeee7", "dark": "#271410"},
}
PAPER = {"light": "#faf7ed", "dark": "#141412"}


def _lin(c):
    c /= 255
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def _rgb(hex_):
    h = hex_.lstrip("#")
    return tuple(_lin(int(h[i : i + 2], 16)) for i in (0, 2, 4))


def _oklab(r, g, b):
    l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b
    m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b
    s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b
    l_, m_, s_ = (max(v, 0) ** (1 / 3) for v in (l, m, s))
    return (
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    )


def oklch(hex_):
    L, a, b = _oklab(*_rgb(hex_))
    return L, math.hypot(a, b), math.degrees(math.atan2(b, a)) % 360


def mix_oklab(fg, bg, pct):
    """What color-mix(in oklab, fg pct%, bg) produces."""
    f, g = _oklab(*_rgb(fg)), _oklab(*_rgb(bg))
    t = pct / 100
    L, a, b = (f[i] * t + g[i] * (1 - t) for i in range(3))
    # OKLab -> linear sRGB -> hex
    l_ = L + 0.3963377774 * a + 0.2158037573 * b
    m_ = L - 0.1055613458 * a - 0.0638541728 * b
    s_ = L - 0.0894841775 * a - 1.2914855480 * b
    l, m, s = l_**3, m_**3, s_**3
    lr = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
    lg = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
    lb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s

    def unlin(c):
        c = min(max(c, 0.0), 1.0)
        return 12.92 * c if c <= 0.0031308 else 1.055 * (c ** (1 / 2.4)) - 0.055

    return "#%02x%02x%02x" % tuple(round(unlin(v) * 255) for v in (lr, lg, lb))


def contrast(a, b):
    def rel(hex_):
        r, g, bl = _rgb(hex_)
        return 0.2126 * r + 0.7152 * g + 0.0722 * bl

    la, lb = rel(a), rel(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


def wedge_distance(hue):
    """Degrees outside the Forge hue wedge; 0 if inside it."""
    lo, hi = FORGE_WEDGE
    if lo <= hue <= hi:
        return 0.0
    return min(
        min(abs(hue - e), 360 - abs(hue - e)) for e in (lo, hi)
    )


def read_theme_blocks(text):
    """The light block, and the explicit-dark block (the auto copy is checked
    against it separately)."""
    blocks = {}
    for name, marker in (
        ("light", r':root\[data-theme="light"\]'),
        ("dark", r':root\[data-theme="dark"\]'),
    ):
        start = re.search(marker, text)
        if not start:
            sys.exit(f"could not find the {name} token block in {BASE_CSS}")
        end = text.index("}", start.end())
        blocks[name] = text[start.end() : end]
    return blocks


def main():
    text = BASE_CSS.read_text()
    blocks = read_theme_blocks(text)
    failures = []

    for theme, block in blocks.items():
        values = {}
        for token in ACCENTS:
            m = re.search(rf"--{re.escape(token)}:\s*(#[0-9a-fA-F]{{6}})", block)
            if not m:
                failures.append(f"{theme}: --{token} is missing")
                continue
            values[token] = m.group(1)

        print(f"\n{theme.upper()}")
        print(
            f"  {'token':<13} {'hex':<9} {'L':>5} {'C':>6} {'H':>6} "
            f"{'off-wedge':>10} {'on tint':>9} {'on badge':>9}"
        )
        Ls = []
        ranked = []
        for token, hex_ in values.items():
            L, C, H = oklch(hex_)
            Ls.append(L)
            off = wedge_distance(H)
            ranked.append((off, C, token))

            tint = mix_oklab(hex_, PAPER[theme], TINT_PCT)
            c_tint = contrast(hex_, tint)
            if c_tint < MIN_CONTRAST:
                failures.append(
                    f"{theme}: --{token} is {c_tint:.2f}:1 on its tint, "
                    f"needs {MIN_CONTRAST}"
                )

            badge = ""
            if token in BADGES:
                c_badge = contrast(hex_, BADGES[token][theme])
                badge = f"{c_badge:.2f}:1"
                if c_badge < MIN_CONTRAST:
                    failures.append(
                        f"{theme}: --{token} is {c_badge:.2f}:1 on its badge "
                        f"surface, needs {MIN_CONTRAST}"
                    )

            print(
                f"  {token:<13} {hex_:<9} {L:.3f} {C:.3f} {H:6.1f} "
                f"{off:9.1f}° {c_tint:8.2f}:1 {badge:>9}"
            )

        if Ls:
            spread = max(Ls) - min(Ls)
            verdict = "ok" if spread <= MAX_L_SPREAD else "TOO WIDE"
            print(f"  lightness spread {spread:.3f} ({verdict})")
            if spread > MAX_L_SPREAD:
                failures.append(
                    f"{theme}: lightness spread {spread:.3f} exceeds "
                    f"{MAX_L_SPREAD} — labels will not read at equal weight"
                )

        # Rule 2: the further a hue sits from the Forge wedge, the less chroma
        # it may carry, against an absolute ceiling.
        #
        # A ceiling rather than a relative ordering, because the defect this
        # guards against is a foreign hue at the *same* chroma as the warm ones,
        # not one above them: the palette's first draft had --co-info at C 0.079,
        # in line with ochre and rust, and it read as a UI status blue dropped on
        # letterpress cream. Only draining it fixed that. These four bands are
        # the design decision, stated numerically so a later hand-edit has
        # something to fail against.
        for off, chroma, token in ranked:
            ceiling = next(
                c for limit, c in CHROMA_CEILING if off <= limit
            )
            # Epsilon absorbs 8-bit hex quantization: a value authored to sit
            # exactly on a ceiling round-trips a hair above it.
            if chroma > ceiling + 0.002:
                failures.append(
                    f"{theme}: --{token} sits {off:.0f}° outside the Forge "
                    f"wedge, where chroma may not exceed {ceiling:.3f}, but "
                    f"carries {chroma:.3f} — drain it or move the hue warmer"
                )

    # The auto block must be a byte-identical copy of dark, or a theme switch
    # silently disagrees with prefers-color-scheme.
    for token in ACCENTS:
        found = re.findall(rf"--{re.escape(token)}:\s*(#[0-9a-fA-F]{{6}})", text)
        if len(found) != 3:
            failures.append(
                f"--{token} appears {len(found)} times; expected 3 "
                f"(light, dark, and the auto copy of dark)"
            )
        elif found[1] != found[2]:
            failures.append(
                f"--{token}: the auto block says {found[2]} but dark says "
                f"{found[1]}"
            )

    if failures:
        print("\nFAILED:")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("\nAll palette rules hold.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
