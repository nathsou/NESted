"""Build NES pattern tables (CHR) from ASCII-art tiles.

Pixels: '.' or ' ' = color 0, '1'/'a'/'+' = 1, '2'/'b'/'o' = 2, '3'/'c'/'#' = 3.
A tile is 8 rows of 8 characters; a 16x16 metatile is 16 rows of 16.
"""
import os
from font import FONT

PIXELS = {'.': 0, ' ': 0, '0': 0, '1': 1, 'a': 1, '+': 1, '2': 2, 'b': 2, 'o': 2, '3': 3, 'c': 3, '#': 3}


def encode(rows):
    """16 bytes of one 8x8 tile."""
    assert len(rows) == 8 and all(len(r) == 8 for r in rows), rows
    lo, hi = [], []
    for r in rows:
        a = b = 0
        for x, ch in enumerate(r):
            v = PIXELS[ch]
            a |= (v & 1) << (7 - x)
            b |= ((v >> 1) & 1) << (7 - x)
        lo.append(a)
        hi.append(b)
    return bytes(lo + hi)


def split16(rows):
    """Split a 16x16 drawing into four tiles: top-left, top-right, bottom-left, bottom-right."""
    assert len(rows) == 16 and all(len(r) == 16 for r in rows), rows
    return [[r[x:x + 8] for r in rows[y:y + 8]] for y in (0, 8) for x in (0, 8)]


def flip(rows):
    return [r[::-1] for r in rows]


def recolor(rows, mapping):
    return [''.join(mapping.get(c, c) for c in r) for r in rows]


class Table:
    """A 4 KiB pattern table of 256 tiles."""

    def __init__(self):
        self.tiles = [bytes(16)] * 256
        self.names = {}
        self.next = 1

    def font(self, color='#'):
        for ch, rows in FONT.items():
            self.tiles[ord(ch)] = encode([r.replace('#', color) for r in rows])

    def put(self, index, rows, name=None):
        self.tiles[index] = encode(rows)
        if name:
            self.names[name] = index
        return index

    def add(self, rows, name=None):
        while self.next < 256 and (0x20 <= self.next < 0x60 and any(self.tiles[0x20:0x60])) :
            self.next = 0x60
        i = self.next
        self.next += 1
        return self.put(i, rows, name)

    def add16(self, rows, name=None):
        """Add a 16x16 metatile as four consecutive tiles; returns the first index."""
        parts = split16(rows)
        first = self.add(parts[0])
        for p in parts[1:]:
            self.add(p)
        if name:
            self.names[name] = first
        return first

    def data(self):
        return b''.join(self.tiles)


def write(path, *tables):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    data = b''.join(t.data() for t in tables)
    with open(path, 'wb') as f:
        f.write(data)
    return data


def preview(path, data, palette=((0, 0, 0), (90, 90, 90), (170, 170, 170), (255, 255, 255)), scale=3):
    """Write a PNG contact sheet of CHR data (needs Pillow)."""
    from PIL import Image
    tiles = len(data) // 16
    cols = 16
    rows = (tiles + cols - 1) // cols
    img = Image.new('RGB', (cols * 8 * scale, rows * 8 * scale))
    for t in range(tiles):
        tx, ty = t % cols, t // cols
        for y in range(8):
            lo, hi = data[t * 16 + y], data[t * 16 + 8 + y]
            for x in range(8):
                c = ((lo >> (7 - x)) & 1) | (((hi >> (7 - x)) & 1) << 1)
                for sy in range(scale):
                    for sx in range(scale):
                        img.putpixel(((tx * 8 + x) * scale + sx, (ty * 8 + y) * scale + sy), palette[c])
    img.save(path)
