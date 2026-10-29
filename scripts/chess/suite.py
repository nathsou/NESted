#!/usr/bin/env python3
"""Tactical test suite: build it with Stockfish, or score the cartridge on it.

  scripts/chess/suite.py build --count 300 > suite.epd
  scripts/chess/suite.py run suite.epd --frames 300 [--source games/chess.nst]

A position qualifies when Stockfish's best move beats its second choice by at
least --gap centipawns: one clear solution, usually tactical.
"""
import argparse
import pathlib
import random
import subprocess
import sys

import chess
import chess.engine

ROOT = pathlib.Path(__file__).resolve().parents[2]
BENCH = ROOT / "target" / "release" / "chessbench"


def build(args):
    rng = random.Random(args.seed)
    engine = chess.engine.SimpleEngine.popen_uci(args.stockfish)
    engine.configure({"Threads": 1, "Hash": 64})
    weak = chess.engine.SimpleEngine.popen_uci(args.stockfish)
    weak.configure({"Threads": 1, "Skill Level": 3})
    found = 0
    while found < args.count:
        board = chess.Board()
        for _ in range(rng.randint(8, 16)):
            if board.is_game_over():
                break
            board.push(rng.choice(list(board.legal_moves)))
        while not board.is_game_over() and board.ply() < 160 and found < args.count:
            if board.ply() >= 16 and rng.random() < 0.35:
                infos = engine.analyse(board, chess.engine.Limit(depth=args.depth), multipv=2)
                if len(infos) == 2:
                    a = infos[0]["score"].relative.score(mate_score=3000)
                    b = infos[1]["score"].relative.score(mate_score=3000)
                    if a - b >= args.gap and -200 <= a <= 2500:
                        print(f"{board.fen()};{infos[0]['pv'][0].uci()}", flush=True)
                        found += 1
            board.push(weak.play(board, chess.engine.Limit(time=0.02)).move)
    engine.quit()
    weak.quit()


def run(args):
    lines = [l.strip() for l in open(args.file) if l.strip()]
    fens = [l.split(";")[0] for l in lines]
    moves = [l.split(";")[1] for l in lines]
    out = subprocess.run(
        [str(BENCH), "go", args.source, "0", str(args.frames)],
        input="\n".join(fens) + "\n", capture_output=True, text=True,
    ).stdout.split("\n")
    solved = sum(1 for o, m in zip(out, moves) if o.split() and o.split()[0] == m)
    print(f"{solved}/{len(lines)} solved at {args.frames} frames")


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--count", type=int, default=300)
    b.add_argument("--depth", type=int, default=14)
    b.add_argument("--gap", type=int, default=200)
    b.add_argument("--seed", type=int, default=11)
    b.add_argument("--stockfish", default="/usr/games/stockfish")
    r = sub.add_parser("run")
    r.add_argument("file")
    r.add_argument("--frames", type=int, default=300)
    r.add_argument("--source", default=str(ROOT / "games" / "chess.nst"))
    args = p.parse_args()
    build(args) if args.cmd == "build" else run(args)


if __name__ == "__main__":
    main()
