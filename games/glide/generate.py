"""Search for good Glide levels: random rooms filtered by the solver.

Quality: optimal solution length, required mechanics, and states explored.
Run `python3 generate.py CHAPTER SEED COUNT` to print candidates.
"""
import random, sys
from levels import Level, W, H


def room(rng, w, h, rocks, snow, holes, crates, stars):
    g = [['#'] * W for _ in range(H)]
    x0 = (W - w) // 2
    y0 = (H - h) // 2
    cells = []
    for y in range(y0, y0 + h):
        for x in range(x0, x0 + w):
            g[y][x] = '.'
            cells.append((x, y))
    def place(ch, n):
        free = [c for c in cells if g[c[1]][c[0]] == '.']
        rng.shuffle(free)
        for (x, y) in free[:n]:
            g[y][x] = ch
    place('#', rocks)
    place(',', snow)
    place('O', holes)
    place('B', crates)
    place('*', stars)
    place('E', 1)
    place('@', 1)
    return [''.join(r) for r in g]


def events(lv, sol):
    """Count mechanics used along a solution."""
    s = lv.start()
    snow_stops = pushes = fills = 0
    for d in sol:
        t, _ = lv.step(s, d)
        if t[2] != s[2]:
            pushes += 1
        if len(t[3]) > len(s[3]):
            fills += 1
        if lv.floor[t[0]] == ',':
            snow_stops += 1
        s = t
    return snow_stops, pushes, fills


def without(rows, ch, repl):
    return [r.replace(ch, repl) for r in rows]


CHAPTERS = {
    # name: (w range, h range, rocks, snow, holes, crates, stars, need)
    1: ((6, 10), (5, 8), (3, 9), (0, 0), (0, 0), (0, 0), (2, 4), None),
    2: ((7, 12), (6, 9), (3, 10), (2, 6), (0, 0), (0, 0), (2, 5), ','),
    3: ((8, 12), (6, 9), (3, 10), (0, 4), (2, 5), (0, 0), (2, 5), 'O'),
    4: ((7, 12), (6, 9), (3, 9), (0, 3), (0, 2), (1, 2), (2, 4), 'B'),
    5: ((9, 14), (7, 11), (5, 12), (1, 5), (1, 4), (1, 3), (3, 6), 'BO'),
}


def candidate(rng, chapter):
    (w0, w1), (h0, h1), r, sn, ho, cr, st, need = CHAPTERS[chapter]
    w, h = rng.randint(w0, w1), rng.randint(h0, h1)
    rows = room(rng, w, h, rng.randint(*r), rng.randint(*sn), rng.randint(*ho), rng.randint(*cr), rng.randint(*st))
    lv = Level(rows)
    sol = lv.solve(limit=60000)
    if sol is None:
        return None
    snow_stops, pushes, fills = events(lv, sol)
    # The chapter's mechanic must matter.
    if need:
        for ch in need:
            if ch == ',' and snow_stops == 0:
                return None
            if ch == 'B' and pushes == 0:
                return None
            if ch == 'O' and 'B' in need and fills == 0:
                return None
        if ',' in need and Level(without(rows, ',', '.')).solve(limit=60000) is not None:
            return None
        if 'B' in need and Level(without(rows, 'B', '.')).solve(limit=60000) is not None:
            return None
    score = len(sol)
    return score, rows, sol


def main():
    chapter = int(sys.argv[1])
    rng = random.Random(int(sys.argv[2]))
    count = int(sys.argv[3])
    lo, hi = int(sys.argv[4]), int(sys.argv[5])
    best = []
    tries = 0
    while len(best) < count and tries < 40000:
        tries += 1
        c = candidate(rng, chapter)
        if c and lo <= c[0] <= hi:
            best.append(c)
            print(f"# {c[0]} moves: {c[2]}")
            print("    [")
            for r in c[1]:
                print(f'        "{r}",')
            print("    ],")
            sys.stdout.flush()


if __name__ == '__main__':
    main()
