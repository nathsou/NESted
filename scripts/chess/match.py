#!/usr/bin/env python3
"""Plays the NESted Chess cartridge against Stockfish and estimates Elo.

The cartridge runs inside chessbench's headless 6502 core and thinks for a
fixed number of NES frames per move (600 frames = 10 s on a real console).
Stockfish plays with UCI_LimitStrength at a given UCI_Elo, or at a fixed
Skill Level. Each opening is played twice with colors reversed.

  scripts/chess/match.py --games 40 --frames 600 --elo 1500
"""
import argparse
import concurrent.futures
import math
import os
import pathlib
import random

import chess
import chess.engine
import chess.pgn

ROOT = pathlib.Path(__file__).resolve().parents[2]
BENCH = ROOT / "target" / "release" / "chessbench"

# Short, balanced, common openings (UCI moves from the initial position).
OPENINGS = [
    "e2e4 e7e5 g1f3 b8c6 f1b5 a7a6",
    "e2e4 e7e5 g1f3 b8c6 f1c4 f8c5",
    "e2e4 e7e5 g1f3 b8c6 d2d4 e5d4",
    "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4",
    "e2e4 c7c5 g1f3 b8c6 d2d4 c5d4",
    "e2e4 c7c5 b1c3 b8c6 g2g3 g7g6",
    "e2e4 e7e6 d2d4 d7d5 b1c3 g8f6",
    "e2e4 e7e6 d2d4 d7d5 e4e5 c7c5",
    "e2e4 c7c6 d2d4 d7d5 b1c3 d5e4",
    "e2e4 c7c6 d2d4 d7d5 e4e5 c8f5",
    "e2e4 d7d6 d2d4 g8f6 b1c3 g7g6",
    "e2e4 g8f6 e4e5 f6d5 d2d4 d7d6",
    "e2e4 d7d5 e4d5 d8d5 b1c3 d5a5",
    "d2d4 d7d5 c2c4 e7e6 b1c3 g8f6",
    "d2d4 d7d5 c2c4 c7c6 g1f3 g8f6",
    "d2d4 d7d5 c2c4 d5c4 g1f3 g8f6",
    "d2d4 g8f6 c2c4 e7e6 b1c3 f8b4",
    "d2d4 g8f6 c2c4 g7g6 b1c3 f8g7",
    "d2d4 g8f6 c2c4 e7e6 g1f3 b7b6",
    "d2d4 g8f6 c2c4 c7c5 d4d5 e7e6",
    "d2d4 d7d5 g1f3 g8f6 c1f4 e7e6",
    "d2d4 f7f5 g2g3 g8f6 f1g2 g7g6",
    "c2c4 e7e5 b1c3 g8f6 g1f3 b8c6",
    "c2c4 g8f6 b1c3 e7e6 e2e4 d7d5",
    "c2c4 c7c5 g1f3 g8f6 b1c3 b8c6",
    "g1f3 d7d5 g2g3 g8f6 f1g2 e7e6",
    "g1f3 g8f6 c2c4 g7g6 b1c3 f8g7",
    "e2e4 e7e5 f2f4 e5f4 g1f3 g7g5",
    "e2e4 e7e5 b1c3 g8f6 f1c4 b8c6",
    "e2e4 b8c6 d2d4 d7d5 b1c3 d5e4",
    "b2b3 e7e5 c1b2 b8c6 e2e3 g8f6",
    "e2e4 g7g6 d2d4 f8g7 b1c3 d7d6",
]


def play_game(job):
    index, opening, nes_white, args = job
    nes = chess.engine.SimpleEngine.popen_uci([str(BENCH), "uci", str(args.source)])
    sf = chess.engine.SimpleEngine.popen_uci(args.stockfish)
    try:
        nes.configure({"Frames": args.frames})
        options = {"Threads": 1, "Hash": 16}
        if args.skill is not None:
            options["Skill Level"] = args.skill
        else:
            options.update({"UCI_LimitStrength": True, "UCI_Elo": args.elo})
        sf.configure(options)
        board = chess.Board()
        for m in opening.split():
            board.push_uci(m)
        nes.protocol.send_line("ucinewgame")
        while not board.is_game_over(claim_draw=True) and board.ply() < 400:
            nes_turn = (board.turn == chess.WHITE) == nes_white
            if nes_turn:
                result = nes.play(board, chess.engine.Limit(time=3600))
            else:
                result = sf.play(board, chess.engine.Limit(time=args.sf_time))
            if result.move is None or result.move not in board.legal_moves:
                # An illegal or missing move forfeits.
                winner = not board.turn
                return index, nes_white, (1.0 if (winner == chess.WHITE) == nes_white else 0.0), board, f"illegal {result.move}"
            board.push(result.move)
        outcome = board.outcome(claim_draw=True)
        if outcome is None or outcome.winner is None:
            score = 0.5
        else:
            score = 1.0 if (outcome.winner == chess.WHITE) == nes_white else 0.0
        return index, nes_white, score, board, outcome.termination.name if outcome else "length"
    finally:
        nes.quit()
        sf.quit()


def elo(score, n):
    p = min(max(score / n, 1e-3), 1 - 1e-3)
    diff = -400 * math.log10(1 / p - 1)
    # Standard error of the mean score -> Elo error (95%).
    s = math.sqrt(p * (1 - p) / n)
    lo = -400 * math.log10(1 / min(max(p - 1.96 * s, 1e-3), 1 - 1e-3) - 1)
    hi = -400 * math.log10(1 / min(max(p + 1.96 * s, 1e-3), 1 - 1e-3) - 1)
    return diff, lo, hi


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--games", type=int, default=32)
    a.add_argument("--frames", type=int, default=600)
    a.add_argument("--elo", type=int, default=1500)
    a.add_argument("--skill", type=int)
    a.add_argument("--sf-time", type=float, default=0.1)
    a.add_argument("--stockfish", default="/usr/games/stockfish")
    a.add_argument("--source", default=str(ROOT / "games" / "chess.nst"))
    a.add_argument("--jobs", type=int, default=os.cpu_count())
    a.add_argument("--pgn")
    a.add_argument("--seed", type=int, default=1)
    args = a.parse_args()
    rng = random.Random(args.seed)
    openings = OPENINGS[:]
    rng.shuffle(openings)
    jobs = []
    for i in range(args.games):
        opening = openings[(i // 2) % len(openings)]
        jobs.append((i, opening, i % 2 == 0, args))
    total = 0.0
    done = 0
    pgn = open(args.pgn, "w") if args.pgn else None
    with concurrent.futures.ProcessPoolExecutor(args.jobs) as pool:
        for index, nes_white, score, board, why in pool.map(play_game, jobs):
            total += score
            done += 1
            if pgn:
                game = chess.pgn.Game.from_board(board)
                game.headers["White"] = "NESted" if nes_white else "Stockfish"
                game.headers["Black"] = "Stockfish" if nes_white else "NESted"
                game.headers["Round"] = str(index)
                print(game, file=pgn, end="\n\n")
            print(f"game {index:3} {'W' if nes_white else 'B'} {score:.1f} {why:22} plies {board.ply():3}  running {total}/{done}", flush=True)
    d, lo, hi = elo(total, done)
    opponent = f"skill {args.skill}" if args.skill is not None else f"UCI_Elo {args.elo}"
    print(f"\n{total}/{done} vs Stockfish {opponent}: Elo diff {d:+.0f} (95% {lo:+.0f}..{hi:+.0f})")
    if args.skill is None:
        print(f"estimated NESted Elo ≈ {args.elo + d:.0f}")


if __name__ == "__main__":
    main()
