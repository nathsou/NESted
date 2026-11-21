"""Starter graphics for new playground projects: the font, a few
background tiles and 16x16 sprites. Writes playground/public/templates/."""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from tiles import Table, split16, write

BLOCK = [
    "aaaaaaaaaaaaaaaa", "abbbbbbbbbbbbbbc", "abcccccccccccccc", "abcbbbbbbbbbbbbc",
    "abcbbbbbbbbbbbbc", "abcbbbbbbbbbbbbc", "abcbbbbbbbbbbbbc", "abcbbbbbbbbbbbbc",
    "abcbbbbbbbbbbbbc", "abcbbbbbbbbbbbbc", "abcbbbbbbbbbbbbc", "abcbbbbbbbbbbbbc",
    "abcbbbbbbbbbbbbc", "abcbbbbbbbbbbbbc", "abccccccccccccca", "accccccccccccccc",
]
CHECKER = ["".join('b' if ((x // 8) + (y // 8)) % 2 else 'a' for x in range(16)) for y in range(16)]
STAR = [
    "................", ".......cc.......", ".......cc.......", "......cbbc......",
    "......cbbc......", ".cccccbbbbccccc.", "..cbbbbbbbbbbc..", "...cbbbbbbbbc...",
    "....cbbbbbbc....", "....cbbbbbbc....", "...cbbbccbbbc...", "...cbbc..cbbc...",
    "..cbc......cbc..", "..cc........cc..", "................", "................",
]
BALL = [
    "................", ".....cccccc.....", "...ccbbbbbbcc...", "..cbbbbbbbbbbc..",
    "..cbaabbbbbbbc..", ".cbaabbbbbbbbbc.", ".cbabbbbbbbbbbc.", ".cbbbbbbbbbbbbc.",
    ".cbbbbbbbbbbbbc.", ".cbbbbbbbbbbbbc.", ".cbbbbbbbbbbbbc.", "..cbbbbbbbbbbc..",
    "..cbbbbbbbbbbc..", "...ccbbbbbbcc...", ".....cccccc.....", "................",
]
SHIP = [
    ".......cc.......", ".......cc.......", "......cbbc......", "......cbbc......",
    ".....cbaabc.....", ".....cbaabc.....", "....cbbaabbc....", "...cbbbbbbbbc...",
    "..cbbbbbbbbbbc..", ".cbbbccbbccbbbc.", "cbbbc.cbbc.cbbbc", "cbbc..cbbc..cbbc",
    "cbc...caac...cbc", "cc....caac....cc", "......cbbc......", ".......cc.......",
]


def tables(invert=False):
    bg = Table()
    bg.font('c')
    at = 0x60
    for rows in (BLOCK, CHECKER, STAR):
        if invert:
            rows = [r.replace('b', 'x').replace('c', 'b').replace('x', 'c') for r in rows]
        for i, p in enumerate(split16(rows)):
            bg.put(at + i, p)
        at += 4
    sp = Table()
    at = 0
    for rows in (BALL, SHIP, STAR):
        for i, p in enumerate(split16(rows)):
            sp.put(at + i, p)
        at += 4
    return bg, sp


if __name__ == '__main__':
    out = os.path.join(os.path.dirname(__file__), '..', '..', 'playground', 'public', 'templates')
    write(os.path.join(out, 'tiles.chr'), *tables())
    write(os.path.join(out, 'tiles-alt.chr'), *tables(True))
    print('templates written')
