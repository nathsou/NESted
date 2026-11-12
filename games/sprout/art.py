"""Sprout graphics for an MMC3 board.

CHR layout (1 KiB MMC3 banks):
  $0000-$07FF  background tiles $00-$7F: font and blanks (R0 = 0)
  $0800-$0FFF  background tiles $80-$FF, animation frame 0 (R1 = 2)
  $1000-$1FFF  sprites (R2-R5 = 4-7)
  $2000/$2800/$3000  background $80-$FF, frames 1-3 (R1 = 8, 10, 12)
"""
import math, os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..', 'scripts', 'art'))
from tiles import Table, encode, split16, flip, write, preview

# ------------------------------------------------------------ background
GRASS = [
    "cccccccccccccccc",
    "cccbcccccccbcccc",
    "ccbcccbcccbccbcc",
    "bbccccbbbbcccbbb",
    "aabbbbaaaabbbbaa",
    "bbaabbbbbbbaabbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbabbbbbbbbbb",
    "bbbbbbbbbbbbbabb",
    "bbabbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbabbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "babbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbab",
    "bbbbbbbbbbbbbbbb",
]
DIRT = [
    "bbbbbbbbbbbbbbbb",
    "bbbbbabbbbbbbbbb",
    "bbbbbbbbbbbbbabb",
    "bbabbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbabbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "babbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbab",
    "bbbbbbbbbbbbbbbb",
    "bbbbbabbbbbbbbbb",
    "bbbbbbbbbbbbbabb",
    "bbabbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbabbbbbbb",
    "bbbbbbbbbbbbbbbb",
]
BRICK = [
    "aaaaaaaaaaaaaaaa",
    "bbbbbbbacbbbbbba",
    "bbbbbbbacbbbbbba",
    "bbbbbbbacbbbbbba",
    "aaaaaaaaaaaaaaaa",
    "bbbacbbbbbbbacbb",
    "bbbacbbbbbbbacbb",
    "bbbacbbbbbbbacbb",
    "aaaaaaaaaaaaaaaa",
    "bbbbbbbacbbbbbba",
    "bbbbbbbacbbbbbba",
    "bbbbbbbacbbbbbba",
    "aaaaaaaaaaaaaaaa",
    "bbbacbbbbbbbacbb",
    "bbbacbbbbbbbacbb",
    "bbbacbbbbbbbacbb",
]
STONE = [
    "cccccccccccccccb",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbcbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "cbbbbbbbbbbbbbba",
    "baaaaaaaaaaaaaaa",
]
PLANK = [  # one-way platform
    "cccccccccccccccc",
    "bbbbbbbbbbbbbbbb",
    "bbbabbbbbbbabbbb",
    "aaaaaaaaaaaaaaaa",
    "..a..........a..",
    "..a..........a..",
    "...a........a...",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
]
SPIKES = [
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "...c.......c....",
    "...c.......c....",
    "..cca.....cca...",
    "..cca.....cca...",
    ".ccaaa...ccaaa..",
    ".ccaaa...ccaaa..",
    "cccaaaa.cccaaaa.",
    "cccaaaa.cccaaaa.",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
]


def spring(phase):
    squash = [0, 1, 2, 1][phase]
    rows = ["................"] * (4 + squash)
    rows += [
        "..bbbbbbbbbbbb..",
        ".baaaaaaaaaaaab.",
        "..bbbbbbbbbbbb..",
    ]
    coil = ["....bbbbbbbb....", "...b........b...", "....bbbbbbbb....", "...b........b..."]
    while len(rows) < 13:
        rows.append(coil[len(rows) % 4])
    rows += [
        "..aaaaaaaaaaaa..",
        ".abbbbbbbbbbbba.",
        "aaaaaaaaaaaaaaaa",
    ]
    return rows[:16]


def gem(phase):
    base = [
        "................",
        "................",
        "......bbbb......",
        ".....bccbbb.....",
        "....bccbbbbb....",
        "...bccbbbbbbb...",
        "..bbbbbbbbbbbb..",
        "..abbbbbbbbbba..",
        "...abbbbbbbba...",
        "....abbbbbba....",
        ".....abbbba.....",
        "......abba......",
        ".......aa.......",
        "................",
        "................",
        "................",
    ]
    rows = [list(r) for r in base]
    # A glint sweeps across the gem.
    gx = [4, 7, 10, 13][phase]
    for y in range(2, 12):
        x = gx - (y - 2) // 2
        if 0 <= x < 16 and rows[y][x] == 'b':
            rows[y][x] = 'c'
    return [''.join(r) for r in rows]


def flag(phase):
    rows = []
    for y in range(16):
        r = ['.'] * 16
        r[3] = 'a'
        if y < 9:
            wave = int(round(math.sin((y / 9) * math.pi + phase * math.pi / 2) * 1.5))
            for x in range(4, 13 + wave):
                r[x] = 'b' if (x + y) % 5 else 'c'
        rows.append(''.join(r))
    rows[0] = "...c............"
    return rows


CHECK = [
    "...c............",
    "...accc.........",
    "...acccc........",
    "...accccc.......",
    "...acccc........",
    "...accc.........",
    "...a............",
    "...a............",
    "...a............",
    "...a............",
    "...a............",
    "...a............",
    "...a............",
    "..aaa...........",
    ".aaaaa..........",
    "aaaaaaa.........",
]


def cloud_left():
    rows = []
    for y in range(16):
        r = ''
        for x in range(16):
            d1 = (x - 10) ** 2 + (y - 8) ** 2
            d2 = (x - 15) ** 2 + (y - 5) ** 2
            inside = d1 < 30 or d2 < 26
            r += 'c' if inside and y < 13 else '.'
        rows.append(r)
    return rows


def cloud_right():
    rows = []
    for y in range(16):
        r = ''
        for x in range(16):
            d1 = (x - 5) ** 2 + (y - 8) ** 2
            d2 = (x + 0) ** 2 + (y - 5) ** 2
            inside = d1 < 30 or d2 < 26
            r += 'c' if inside and y < 13 else '.'
        rows.append(r)
    return rows


BUSH = [
    "................",
    "................",
    "................",
    "................",
    "................",
    "......bbbb......",
    "....bbccbbbb....",
    "...bccbbbbccb...",
    "..bcbbbbbbbbbb..",
    "..bbbbbbbbbccbb.",
    ".bbbbbbbbbcbbbb.",
    ".bbbbbbbbbbbbbb.",
    "abbbbbbbbbbbbbba",
    "aabbbbbbbbbbbbaa",
    "aaaaaaaaaaaaaaaa",
    "................",
]
FLOWER = [
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    "................",
    ".......cc.......",
    "......cbbc......",
    "......cbbc......",
    ".......cc.......",
    ".......a........",
    "......aa..aa....",
    ".......a.aa.....",
    ".......aa.......",
    ".......a........",
]

# Metatile names in order; tile numbers are 0x80 + 4 * index.
METATILES = ['GRASS', 'DIRT', 'BRICK', 'STONE', 'PLANK', 'SPIKES', 'SPRING', 'GEM', 'FLAG', 'CHECK',
             'CLOUD_L', 'CLOUD_R', 'BUSH', 'FLOWER']


def metatile(name, phase):
    return {
        'GRASS': GRASS, 'DIRT': DIRT, 'BRICK': BRICK, 'STONE': STONE, 'PLANK': PLANK, 'SPIKES': SPIKES,
        'SPRING': spring(0), 'GEM': gem(phase), 'FLAG': flag(phase), 'CHECK': CHECK,
        'CLOUD_L': cloud_left(), 'CLOUD_R': cloud_right(), 'BUSH': BUSH, 'FLOWER': FLOWER,
    }[name]


def high_bank(phase):
    t = Table()
    for i, name in enumerate(METATILES):
        for k, part in enumerate(split16(metatile(name, phase))):
            t.put((4 * i + k), part)
    # Shifted by 0x80 when used: this table holds tiles $80-$FF.
    return t.data()[:2048]


# --------------------------------------------------------------- sprites
SPROUT_IDLE = [
    "........cc......",
    "......cc.c......",
    ".....c..cc......",
    "......aac.......",
    "......a.........",
    "....aaaaaaa.....",
    "...abbbbbbbba...",
    "..abbbbbbbbbba..",
    "..abbcabbcabba..",
    "..abbaabbaabba..",
    "..abbbbbbbbbba..",
    "..abbbaaaabbba..",
    "...abbbbbbbba...",
    "....aaaaaaaa....",
    "....aa....aa....",
    "...aaa....aaa...",
]
SPROUT_WALK = [
    "........cc......",
    "......cc.c......",
    ".....c..cc......",
    "......aac.......",
    "......a.........",
    "....aaaaaaa.....",
    "...abbbbbbbba...",
    "..abbbbbbbbbba..",
    "..abbcabbcabba..",
    "..abbaabbaabba..",
    "..abbbbbbbbbba..",
    "..abbbaaaabbba..",
    "...abbbbbbbba...",
    "....aaaaaaaa....",
    ".....aa..aa.....",
    "....aaa...aa....",
]
SPROUT_JUMP = [
    ".....c....c.....",
    "......c..c......",
    ".......cc.......",
    ".......a........",
    ".......a........",
    "....aaaaaaa.....",
    "...abbbbbbbba...",
    "..abbbbbbbbbba..",
    "..abbcabbcabba..",
    "..abbaabbaabba..",
    "..abbbbbbbbbba..",
    "..abbbbaabbbba..",
    "...abbbaabbba...",
    "....aaaaaaaa....",
    "...aa......aa...",
    "..aa........aa..",
]
SPROUT_POUND = [
    "................",
    "................",
    "....cc....cc....",
    ".....cc..cc.....",
    "......cccc......",
    "....aaaaaaaa....",
    "..aabbbbbbbbaa..",
    ".abbbbbbbbbbbba.",
    ".abbaaabbaaabba.",
    ".abbbbbbbbbbbba.",
    ".abbbbaaaabbbba.",
    "..abbbbbbbbbba..",
    "...aaaaaaaaaa...",
    "....aa....aa....",
    "...aaaa..aaaa...",
    "................",
]
BEETLE = [
    "................",
    "................",
    "................",
    "................",
    "......aaaa......",
    "....aabbbbaa....",
    "...abbbbcbbba...",
    "..abbbbcbbbbba..",
    "..abbbcbbbbbba..",
    ".aaaaaaaaaaaaaa.",
    "accacbbbbbbbbbba",
    "accaabbbbbbbbbba",
    ".aaaaaaaaaaaaaa.",
    "..a..a....a..a..",
    ".a..a......a..a.",
    "................",
]
BEETLE2 = [
    "................",
    "................",
    "................",
    "................",
    "......aaaa......",
    "....aabbbbaa....",
    "...abbbbcbbba...",
    "..abbbbcbbbbba..",
    "..abbbcbbbbbba..",
    ".aaaaaaaaaaaaaa.",
    "accacbbbbbbbbbba",
    "accaabbbbbbbbbba",
    ".aaaaaaaaaaaaaa.",
    "...a..a..a..a...",
    "...a..a..a..a...",
    "................",
]
FROG = [
    "................",
    "................",
    "................",
    "................",
    "....cc....cc....",
    "...cabc..cabc...",
    "...bbbbbbbbbb...",
    "..bbbbbbbbbbbb..",
    "..bbaaaaaaaabb..",
    ".bbbbbbbbbbbbbb.",
    ".bbbcccccccbbbb.",
    "bbbbccccccccbbbb",
    "bbbbbbbbbbbbbbbb",
    ".bb.bb....bb.bb.",
    "bbb.b......b.bbb",
    "................",
]
FROG_JUMP = [
    "....cc....cc....",
    "...cabc..cabc...",
    "...bbbbbbbbbb...",
    "..bbbbbbbbbbbb..",
    "..bbaaaaaaaabb..",
    ".bbbbbbbbbbbbbb.",
    ".bbbcccccccbbbb.",
    "..bbccccccccbb..",
    "...bbbbbbbbbb...",
    "...bb......bb...",
    "...b........b...",
    "..bb........bb..",
    "..b..........b..",
    ".bb..........bb.",
    "................",
    "................",
]
BAT = [
    "................",
    "................",
    "a..............a",
    "aa............aa",
    "aba...aaaa...aba",
    "abba.abbbba.abba",
    "abbbabcbbcbabbba",
    "abbbbbabbabbbbba",
    ".abbbbbbbbbbbba.",
    "..abb.bbbb.bba..",
    "...a..abba..a...",
    "......a..a......",
    "................",
    "................",
    "................",
    "................",
]
BAT2 = [
    "................",
    "................",
    "................",
    "................",
    "......aaaa......",
    ".....abbbba.....",
    "....abcbbcba....",
    "...abbabbabba...",
    "..abbbbbbbbbba..",
    ".abba.bbbb.abba.",
    "abba..abba..abba",
    "aba...a..a...aba",
    "aa............aa",
    "a..............a",
    "................",
    "................",
]
SPARK = ["...c....", "...c....", "..bcb...", "cccbccc.", "..bcb...", "...c....", "...c....", "........"]
DUST = ["........", "..bb....", ".bccb...", ".bccb.b.", "..bb.bcb", ".....bb.", "........", "........"]
HEART = [".aa.aa..", "abbabba.", "abbbbba.", "abbbbba.", ".abbba..", "..aba...", "...a....", "........"]

SPRITES = [('IDLE', SPROUT_IDLE), ('WALK', SPROUT_WALK), ('JUMP', SPROUT_JUMP), ('POUND', SPROUT_POUND),
           ('BEETLE', BEETLE), ('BEETLE2', BEETLE2), ('FROG', FROG), ('FROG_JUMP', FROG_JUMP),
           ('BAT', BAT), ('BAT2', BAT2), ('GEM', gem(0))]
SMALL = [('SPARK', SPARK), ('DUST', DUST), ('HEART', HEART)]


def sprite_table():
    t = Table()
    at = 0
    names = {}
    for name, rows in SPRITES:
        names[name] = at
        for k, part in enumerate(split16(rows)):
            t.put(at + k, part)
        at += 4
    for name, rows in SMALL:
        names[name] = at
        t.put(at, rows)
        at += 1
    return t.data(), names


def low_table():
    t = Table()
    t.font('c')
    return t.data()[:2048]


if __name__ == '__main__':
    here = os.path.dirname(os.path.abspath(__file__))
    sprites, names = sprite_table()
    data = (low_table() + high_bank(0) + sprites + high_bank(1) + high_bank(2) + high_bank(3))
    data += bytes(16384 - len(data))
    os.makedirs(os.path.join(here, 'assets'), exist_ok=True)
    with open(os.path.join(here, 'assets', 'sprout.chr'), 'wb') as f:
        f.write(data)
    if '--preview' in sys.argv:
        preview(os.path.join(here, 'assets', 'preview.png'), data[:8192])
    for i, n in enumerate(METATILES):
        print(f"M_{n} = {0x80 + 4 * i:#04x}")
    for n, v in names.items():
        print(f"S_{n} = {v:#04x}")
