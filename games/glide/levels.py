"""Glide levels: design, solving and generation.

Legend: '#' rock, '.' ice, ',' snow, '*' star, 'B' ice block, 'O' hole,
'E' exit, '@' start. The grid is 16 x 13 with a rock border.
The cartridge parses the same strings at compile time (see levels.nst).
"""
from collections import deque
import random, sys

W, H = 16, 13
DIRS = {'U': -W, 'D': W, 'L': -1, 'R': 1}


class Level:
    def __init__(self, rows):
        rows = [r for r in rows]
        assert len(rows) == H, (len(rows), rows)
        for r in rows:
            assert len(r) == W, repr(r)
        self.rows = rows
        self.floor = []
        self.stars = []
        self.crates = []
        self.holes = []
        self.player = None
        self.exit = None
        for i, ch in enumerate(''.join(rows)):
            f = {'#': '#', ',': ',', 'O': 'O', 'E': 'E'}.get(ch, '.')
            self.floor.append(f)
            if ch == '*':
                self.stars.append(i)
            elif ch == 'B':
                self.crates.append(i)
            elif ch == 'O':
                self.holes.append(i)
            elif ch == '@':
                self.player = i
            elif ch == 'E':
                self.exit = i
        assert self.player is not None and self.exit is not None

    def start(self):
        return (self.player, frozenset(self.stars), frozenset(self.crates), frozenset())

    def step(self, state, d):
        """Apply a move; returns (new_state, outcome) with outcome None, 'win' or 'fall'."""
        p, stars, crates, filled = state
        delta = DIRS[d]
        open_exit = not stars

        def floor(c):
            if self.floor[c] == 'O' and c in filled:
                return '.'
            return self.floor[c]

        moved = False
        stars = set(stars)
        crates = set(crates)
        while True:
            n = p + delta
            f = floor(n)
            if f == '#' or (f == 'E' and stars):
                break
            if n in crates:
                # Push the block.
                c = n
                while True:
                    m = c + delta
                    fm = floor(m)
                    if fm == '#' or fm == 'E' or m in crates:
                        break
                    c = m
                    if fm == 'O':
                        break
                    if fm == ',':
                        break
                if c != n:
                    crates.discard(n)
                    if floor(c) == 'O':
                        filled = filled | {c}
                    else:
                        crates.add(c)
                    moved = True
                break
            p = n
            moved = True
            if p in stars:
                stars.discard(p)
            if f == 'O':
                return (p, frozenset(stars), frozenset(crates), filled), 'fall'
            if f == 'E':
                return (p, frozenset(stars), frozenset(crates), filled), 'win'
            if f == ',':
                break
        if not moved:
            return None, None
        return (p, frozenset(stars), frozenset(crates), filled), None

    def solve(self, limit=200000):
        start = self.start()
        prev = {start: None}
        q = deque([start])
        while q:
            s = q.popleft()
            for d in 'UDLR':
                t, outcome = self.step(s, d)
                if t is None or outcome == 'fall':
                    continue
                if outcome == 'win':
                    path = [d]
                    while prev[s] is not None:
                        s, m = prev[s]
                        path.append(m)
                    return ''.join(reversed(path))
                if t not in prev:
                    prev[t] = (s, d)
                    q.append(t)
                    if len(prev) > limit:
                        return None
        return None


# Levels in order. Each is 13 rows of 16 characters.
LEVELS = [
    # 1: First slide.
    [
        "################",
        "################",
        "################",
        "################",
        "####........####",
        "####.@....*.####",
        "####........####",
        "####......E.####",
        "################",
        "################",
        "################",
        "################",
        "################",
    ],
    # 2: Rocks stop you.
    [
        "################",
        "################",
        "##############*#",
        "###.@.........*#",
        "###.##########.#",
        "###.#.......E#.#",
        "###.#.########.#",
        "###.#..........#",
        "###.############",
        "###*############",
        "################",
        "################",
        "################",
    ],
]


def check(levels):
    out = []
    for i, rows in enumerate(levels):
        lv = Level(rows)
        sol = lv.solve()
        out.append(sol)
        print(f"level {i + 1:2}: {'UNSOLVABLE' if sol is None else f'{len(sol):2} moves {sol}'}")
    return out


if __name__ == '__main__':
    check(LEVELS)
