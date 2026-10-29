#!/usr/bin/env python3
"""Generates NESted Chess graphics: CHR tiles, palettes and the title screen.

Background tiles (pattern table 0):
  32..95    font, indexed by ASCII code so `text()` writes literal strings
  96..127   decorations
  128..231  board squares: 13 piece kinds x (plain, cursor) x 4 quadrants;
            tile = 128 + (kind * 2 + cursor) * 4 + quadrant (TL, TR, BL, BR).
            kind 0 is empty, 1..6 white P N B R Q K, 7..12 black.
Square colors come from palettes: 0 light, 1 dark, 2/3 highlighted.
"""
import pathlib

OUT = pathlib.Path(__file__).resolve().parents[2] / "games" / "assets"

# '.' square (color 1), '#' body, 'o' outline. White pieces: body 2 (white),
# outline 3 (black). Black pieces: body 3, outline 2.
PIECES = {
    "P": """
................
................
................
................
................
.......oo.......
......o##o......
......o##o......
.......oo.......
......o##o......
.....o####o.....
.....o####o.....
....o######o....
....oooooooo....
................
................""",
    "N": """
................
................
.......o.o......
......o#o#o.....
.....o#####oo...
....o##o#####o..
...o#########o..
..o####o#####o..
...oooo######o..
.....o######o...
....o#######o...
....o#######o...
...o#########o..
...ooooooooooo..
................
................""",
    "B": """
................
.......oo.......
......o##o......
.......oo.......
......o##o......
.....o##o#o.....
.....o#o##o.....
....o#o####o....
.....o####o.....
......o##o......
......o##o......
.....o####o.....
...o########o...
...oooooooooo...
................
................""",
    "R": """
................
................
...ooo.oo.ooo...
...o#ooo#oo#o...
...o#########o..
....o#######o...
.....o#####o....
.....o#####o....
.....o#####o....
.....o#####o....
....o#######o...
...o#########o..
...o#########o..
...ooooooooooo..
................
................""",
    "Q": """
................
..o....oo....o..
.o#o..o##o..o#o.
..o#o.o##o.o#o..
..o#o##oo##o#o..
..o##########o..
...o########o...
...o########o...
....o######o....
....o######o....
...o########o...
..o##########o..
..o##########o..
..oooooooooooo..
................
................""",
    "K": """
................
.......oo.......
......o##o......
.....oo##oo.....
....o######o....
.....oo##oo.....
...oooo##oooo...
..o##########o..
..o###o##o###o..
...o########o...
....o######o....
....o######o....
...o########o...
..o##########o..
..oooooooooooo..
................""",
}
ORDER = "PNBRQK"

FONT = {
    " ": [0, 0, 0, 0, 0, 0, 0],
    "A": [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
    "B": [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
    "C": [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
    "D": [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
    "E": [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
    "F": [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
    "G": [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
    "H": [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
    "I": [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
    "J": [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
    "K": [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
    "L": [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
    "M": [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
    "N": [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
    "O": [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
    "P": [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
    "Q": [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
    "R": [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
    "S": [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
    "T": [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
    "U": [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
    "V": [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
    "W": [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
    "X": [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
    "Y": [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
    "Z": [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
    "0": [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
    "1": [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
    "2": [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
    "3": [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
    "4": [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
    "5": [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
    "6": [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
    "7": [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    "8": [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
    "9": [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
    "!": [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
    "?": [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
    ".": [0, 0, 0, 0, 0, 0x0C, 0x0C],
    ",": [0, 0, 0, 0, 0x0C, 0x04, 0x08],
    ":": [0, 0x0C, 0x0C, 0, 0x0C, 0x0C, 0],
    "-": [0, 0, 0, 0x1F, 0, 0, 0],
    "+": [0, 0x04, 0x04, 0x1F, 0x04, 0x04, 0],
    "=": [0, 0, 0x1F, 0, 0x1F, 0, 0],
    "/": [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
    "#": [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
    "(": [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
    ")": [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
    "<": [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
    ">": [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
    "'": [0x04, 0x04, 0x08, 0, 0, 0, 0],
    "\\": [0x10, 0x10, 0x08, 0x04, 0x02, 0x01, 0x01],
    "|": [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
}

# NES colors: backdrop, then per background palette three colors.
PALETTE = [
    0x0C, 0x37, 0x30, 0x0F,  # light square, white, black
    0x0C, 0x18, 0x30, 0x0F,  # dark square
    0x0C, 0x3B, 0x30, 0x0F,  # highlighted light square
    0x0C, 0x2B, 0x30, 0x0F,  # highlighted dark square
    0x0C, 0x30, 0x16, 0x0F,  # sprites (unused)
    0x0C, 0x30, 0x16, 0x0F,
    0x0C, 0x30, 0x16, 0x0F,
    0x0C, 0x30, 0x16, 0x0F,
]

BOARD_X, BOARD_Y = 2, 6
TEXT_COLOR = 2


def tile_bytes(pixels):
    """8x8 pixel values 0..3 -> 16 bytes of NES planar data."""
    lo, hi = bytearray(8), bytearray(8)
    for y in range(8):
        for x in range(8):
            v = pixels[y][x]
            if v & 1:
                lo[y] |= 0x80 >> x
            if v & 2:
                hi[y] |= 0x80 >> x
    return bytes(lo + hi)


def piece_pixels(kind, cursor):
    grid = [[1] * 16 for _ in range(16)]
    if kind:
        name = ORDER[(kind - 1) % 6]
        black = kind > 6
        art = [r for r in PIECES[name].strip().splitlines()]
        for y, row in enumerate(art):
            for x, c in enumerate(row):
                if c == "#":
                    grid[y][x] = 3 if black else 2
                elif c == "o":
                    grid[y][x] = 2 if black else 3
    if cursor:
        for i in range(5):
            for (x, y) in ((i, 0), (0, i), (15 - i, 0), (15, i), (i, 15), (0, 15 - i),
                           (15 - i, 15), (15, 15 - i)):
                grid[y][x] = 3
            for (x, y) in ((i, 1), (1, i), (15 - i, 1), (14, i), (i, 14), (1, 15 - i),
                           (15 - i, 14), (14, 15 - i)):
                if i >= 1:
                    grid[y][x] = 2
    return grid


def font_tile(rows):
    pixels = [[0] * 8 for _ in range(8)]
    for y, bits in enumerate(rows):
        for x in range(5):
            if bits & (0x10 >> x):
                pixels[y][x + 1] = TEXT_COLOR
    return tile_bytes(pixels)


def chr_rom():
    tiles = [bytes(16)] * 256
    for ch, rows in FONT.items():
        tiles[ord(ch)] = font_tile(rows)
    # 96: solid backdrop-colored, 97: solid color 1 (square fill),
    # 98..99: underline / frame line.
    tiles[97] = tile_bytes([[1] * 8 for _ in range(8)])
    tiles[98] = tile_bytes([[3] * 8 if y == 7 else [0] * 8 for y in range(8)])
    for kind in range(13):
        for cursor in range(2):
            g = piece_pixels(kind, cursor)
            base = 128 + (kind * 2 + cursor) * 4
            for q, (ox, oy) in enumerate(((0, 0), (8, 0), (0, 8), (8, 8))):
                tiles[base + q] = tile_bytes([row[ox:ox + 8] for row in g[oy:oy + 8]])
    data = b"".join(tiles)
    return data + bytes(8192 - len(data))


def nametable():
    nt = bytearray(b" " * 960)

    def text(x, y, s):
        for i, c in enumerate(s):
            nt[y * 32 + x + i] = ord(c)

    text(2, 2, "NESTED CHESS")
    for r in range(8):
        text(1, BOARD_Y + 2 * (7 - r), str(r + 1))
    for f in range(8):
        text(BOARD_X + 2 * f, BOARD_Y + 16, "ABCDEFGH"[f])
    start = "RNBQKBNR"
    for rank in range(8):
        for f in range(8):
            if rank == 0:
                kind = 1 + "PNBRQK".index(start[f])
            elif rank == 1:
                kind = 1
            elif rank == 6:
                kind = 7
            elif rank == 7:
                kind = 7 + "PNBRQK".index(start[f])
            else:
                kind = 0
            base = 128 + kind * 2 * 4
            x = BOARD_X + 2 * f
            y = BOARD_Y + 2 * (7 - rank)
            nt[y * 32 + x] = base
            nt[y * 32 + x + 1] = base + 1
            nt[(y + 1) * 32 + x] = base + 2
            nt[(y + 1) * 32 + x + 1] = base + 3
    attributes = bytearray(64)
    for rank in range(8):
        for f in range(8):
            dark = (rank + f) % 2 == 0
            x = BOARD_X // 2 + f  # 16-pixel block coordinates
            y = BOARD_Y // 2 + (7 - rank)
            index = (y // 2) * 8 + x // 2
            shift = ((y & 1) * 2 + (x & 1)) * 2
            attributes[index] |= (1 if dark else 0) << shift
    return bytes(nt + attributes)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "chess.chr").write_bytes(chr_rom())
    (OUT / "chess.pal").write_bytes(bytes(PALETTE))
    (OUT / "chess.map").write_bytes(nametable())


if __name__ == "__main__":
    main()
