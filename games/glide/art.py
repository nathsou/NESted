"""Glide graphics: four 8 KiB CHR banks that differ only in animated tiles.

Background tiles (pattern table 0) and sprites (pattern table 1) are drawn
as ASCII art: '.' color 0, 'a' color 1, 'b' color 2, 'c' color 3.
"""
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..', 'scripts', 'art'))
from tiles import Table, encode, split16, flip, write, preview
from font import FONT

# ------------------------------------------------------------ background
ICE = [
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbcbbb",
    "bbbbbbbbbbbcbbbb",
    "bbbbbbbbbbcbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbcbbbbbbbbbbbb",
    "bbcbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
]
SNOW = [
    "cccccccccccccccc",
    "cccccccbbccccccc",
    "ccccccbccbcccccc",
    "cccccccccccccccc",
    "ccbbcccccccccbcc",
    "cbccbcccccccbcbc",
    "cccccccccccccccc",
    "cccccccccbbccccc",
    "ccccccccbccbcccc",
    "cccccccccccccccc",
    "cbccccccccccccbc",
    "bcbcccccccccccbc",
    "cccccccbbccccccc",
    "ccccccbccbcccccc",
    "cccccccccccccccc",
    "cccccccccccccccc",
]
ROCK = [
    "bbbbbbbbabbbbbbb",
    "bccbbbbbabbccbbb",
    "bcbbbbbbabbcbbbb",
    "bbbbbbbbabbbbbba",
    "bbbbbbbaabbbbbaa",
    "aaaaaaaa.aaaaaa.",
    "bbbabbbbbbbbabbb",
    "bccabbbbbbbcabbb",
    "bcbabbbbbbbbabbb",
    "bbbabbbbbbbbabbb",
    "bbbaabbbbbbbaabb",
    "aaa.aaaaaaaa.aaa",
    "bbbbbbbabbbbbbbb",
    "bccbbbbabbbccbbb",
    "bbbbbbbabbbbbbbb",
    "aaaaaaa.aaaaaaaa",
]
BLOCK = [  # an ice block, palette 0: deep blue with bright edges
    "cccccccccccccccb",
    "cbbbbbbbbbbbbbaa",
    "cbaaaaaaaaaaaaaa",
    "cbaccaaaaaaaaaaa",
    "cbacaaaaaaaaaaaa",
    "cbaaaaaaaaaaaaaa",
    "cbaaaaaaaaaaaaaa",
    "cbaaaaaaaaaaaaaa",
    "cbaaaaaaaaaaaaaa",
    "cbaaaaaaaaaaaaaa",
    "cbaaaaaaaaaaaaaa",
    "cbaaaaaaaaaacaaa",
    "cbaaaaaaaaacaaaa",
    "cbaaaaaaaaaaaaaa",
    "baaaaaaaaaaaaaaa",
    "baaaaaaaaaaaaaaa",
]
REFROZEN = [  # a hole filled by a block, palette 2 (slides like ice)
    "bbbbbbbbbbbbbbbb",
    "bbbbbcbbbbbbbbbb",
    "bbbbbbcbbbbbbbbb",
    "bbbbbbbcbbbbbabb",
    "bbbbbbbbcbbbabbb",
    "bbbbabbbbbbabbbb",
    "bbbbbabbbbabbbbb",
    "bbbbbbabbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbcbbb",
    "bbbabbbbbbbcbbbb",
    "bbbbabbbbbcbbbbb",
    "bbbbbabbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
]


def water(phase):
    """Dark water with ripples that drift with the animation phase."""
    rows = []
    for y in range(16):
        r = ''
        for x in range(16):
            edge = x in (0, 15) or y in (0, 15)
            ripple = (y % 5 == (phase + x // 4) % 5) and (x + y + phase * 3) % 7 < 3
            r += 'b' if edge else ('c' if ripple else 'a')
        rows.append(r)
    rows[0] = 'b' * 16
    rows[1] = 'b' + 'a' * 14 + 'b'
    return rows


def star(phase):
    """A gold star on ice (palette 3: a ice, b gold, c white)."""
    base = [
        "aaaaaaaaaaaaaaaa",
        "aaaaaaabbaaaaaaa",
        "aaaaaaabbaaaaaaa",
        "aaaaaabbbbaaaaaa",
        "aaaaaabcbbaaaaaa",
        "abbbbbbcbbbbbbba",
        "aabbbbbbbbbbbbaa",
        "aaabbbbbbbbbbaaa",
        "aaaabbbbbbbbaaaa",
        "aaaabbbbbbbbaaaa",
        "aaabbbbaabbbbaaa",
        "aaabbbaaaabbbaaa",
        "aabbaaaaaaaabbaa",
        "aabaaaaaaaaaabaa",
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa",
    ]
    rows = [list(r) for r in base]
    sparkle = [(2, 12), (12, 3), (13, 13), (3, 2)][phase]
    y, x = sparkle
    for dy, dx in ((0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)):
        rows[y + dy][x + dx] = 'c'
    return [''.join(r) for r in rows]


def exit_tile(open_, phase):
    """The exit hatch: closed (dark with a lock) or open (a golden swirl)."""
    rows = []
    for y in range(16):
        r = ''
        for x in range(16):
            dx, dy = x - 7.5, y - 7.5
            d2 = dx * dx + dy * dy
            if d2 > 56:
                r += 'a'
            elif d2 > 40:
                r += 'c'
            elif not open_:
                r += '.'
            else:
                import math
                ang = math.atan2(dy, dx) + phase * math.pi / 2
                band = int((ang * 2 / math.pi + math.sqrt(d2) / 2.5)) % 2
                r += 'b' if band else 'c'
        rows.append(r)
    if not open_:
        rows = [list(r) for r in rows]
        for (y, x) in [(6, 6), (6, 7), (6, 8), (6, 9), (7, 6), (7, 9), (8, 5), (8, 6), (8, 7), (8, 8), (8, 9), (8, 10),
                       (9, 5), (9, 6), (9, 7), (9, 8), (9, 9), (9, 10), (10, 5), (10, 6), (10, 7), (10, 8), (10, 9), (10, 10)]:
            rows[y][x] = 'b'
        rows = [''.join(r) for r in rows]
    return rows


def big_letter(ch):
    """A 16x16 title letter from the font: gold with a white top and dark outline."""
    g = FONT[ch]
    on = lambda x, y: 0 <= x < 8 and 0 <= y < 8 and g[y][x] == '#'
    rows = []
    for y in range(16):
        r = ''
        for x in range(16):
            gx, gy = x // 2, y // 2
            if on(gx, gy):
                r += 'c' if (y % 2 == 0 and not on(gx, gy - 1)) else 'b'
            elif on((x - 1) // 2, (y - 1) // 2) or on((x + 1) // 2, (y + 1) // 2) and False:
                r += '.'
            else:
                r += 'a'
        rows.append(r)
    return rows


# --------------------------------------------------------------- sprites
PENGUIN_DOWN = [
    "......aaaa......",
    "....aaaaaaaa....",
    "...aaaaaaaaaa...",
    "..aaabbaabbaaa..",
    "..aaab.aab.aaa..",
    "..aabbbccbbbaa..",
    ".aaabbccccbbaaa.",
    ".aaabbbccbbbaaa.",
    "aa.abbbbbbbba.aa",
    "a..abbbbbbbba..a",
    "...abbbbbbbba...",
    "...aabbbbbbaa...",
    "....aabbbbaa....",
    ".....aaaaaa.....",
    "....cccaaccc....",
    "................",
]
PENGUIN_UP = [
    "......aaaa......",
    "....aaaaaaaa....",
    "...aaaaaaaaaa...",
    "..aaaaaaaaaaaa..",
    "..aaaaaaaaaaaa..",
    "..aaaaaaaaaaaa..",
    ".aaaaaaaaaaaaaa.",
    ".aaaaaaaaaaaaaa.",
    "aa.aaaaaaaaaa.aa",
    "a..aaaaaaaaaa..a",
    "...aaaaaaaaaa...",
    "...aaaaaaaaaa...",
    "....aaaaaaaa....",
    ".....aaaaaa.....",
    "....cccaaccc....",
    "................",
]
PENGUIN_LEFT = [
    "......aaaa......",
    "....aaaaaaa.....",
    "...aabaaaaaa....",
    "..aab.aaaaaa....",
    ".cccbbaaaaaaa...",
    "..ccbbbaaaaaa...",
    "...abbbbaaaaa...",
    "...abbbbbaaaaa..",
    "...abbbbbaaaaaa.",
    "...abbbbbaaaa.aa",
    "...abbbbbaaaa...",
    "...aabbbbaaaa...",
    "....aabbaaaa....",
    ".....aaaaaa.....",
    "....cccaccc.....",
    "................",
]
SMALL = [  # falling
    "................",
    "................",
    "................",
    "................",
    "......aaaa......",
    ".....aaaaaa.....",
    "....aab..baa....",
    "....aabccbaa....",
    "....aabbbbaa....",
    ".....abbbba.....",
    "......aaaa......",
    "................",
    "................",
    "................",
    "................",
    "................",
]
SPARK = [
    "...c....",
    "...c....",
    "..bcb...",
    "cccbccc.",
    "..bcb...",
    "...c....",
    "...c....",
    "........",
]
PUFF = [
    "........",
    "..bb....",
    ".bccb.b.",
    ".bccb...",
    "..bb.bb.",
    "....bccb",
    "....bccb",
    ".....bb.",
]
ARROW = [
    "........",
    "..c.....",
    "..cc....",
    "..ccc...",
    "..cccc..",
    "..ccc...",
    "..cc....",
    "..c.....",
]

# Tile numbers shared with the program (see main.nst).
TILES = {}


def bank(phase):
    bg = Table()
    bg.font('c')
    # Metatiles from 0x60, four tiles each (TL, TR, BL, BR).
    order = [
        ('ICE', ICE), ('SNOW', SNOW), ('ROCK', ROCK), ('HOLE', water(phase)), ('BLOCK', BLOCK),
        ('STAR', star(phase)), ('EXIT', exit_tile(False, phase)), ('OPEN', exit_tile(True, phase)),
        ('REFROZEN', REFROZEN),
    ]
    at = 0x60
    for name, rows in order:
        TILES[name] = at
        for i, part in enumerate(split16(rows)):
            bg.put(at + i, part)
        at += 4
    for ch in 'GLIDE':
        TILES['BIG_' + ch] = at
        for i, part in enumerate(split16(big_letter(ch))):
            bg.put(at + i, part)
        at += 4
    # Solid fills for frames and the HUD.
    TILES['DARK'] = 0x20  # space glyph: color 0
    spr = Table()
    sat = 0
    for name, rows in [('P_DOWN', PENGUIN_DOWN), ('P_UP', PENGUIN_UP), ('P_LEFT', PENGUIN_LEFT),
                       ('P_RIGHT', flip(PENGUIN_LEFT)), ('P_SMALL', SMALL), ('BLOCK_S', BLOCK)]:
        TILES[name] = sat
        for i, part in enumerate(split16(rows)):
            spr.put(sat + i, part)
        sat += 4
    for name, rows in [('SPARK', SPARK), ('PUFF', PUFF), ('ARROW', ARROW)]:
        TILES[name] = sat
        spr.put(sat, rows)
        sat += 1
    return bg, spr


if __name__ == '__main__':
    here = os.path.dirname(os.path.abspath(__file__))
    tables = []
    for phase in range(4):
        tables.extend(bank(phase))
    data = write(os.path.join(here, 'assets', 'glide.chr'), *tables)
    if '--preview' in sys.argv:
        preview(os.path.join(here, 'assets', 'preview.png'), data[:8192])
    for k, v in sorted(TILES.items(), key=lambda kv: kv[1]):
        print(f"{k} = {v:#04x}")
