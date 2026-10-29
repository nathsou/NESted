#!/usr/bin/env python3
"""Self-play between two builds of the NESted Chess cartridge.

Each opening is played twice with colors reversed. Prints the score of
build A against build B and the implied Elo difference.

  scripts/chess/selfplay.py --a games/chess.nst --b /tmp/old/chess.nst --games 100
"""
import argparse
import concurrent.futures
import os
import random

import chess
import chess.engine

from match import BENCH, OPENINGS, elo


def play(job):
    index, opening, a_white, args = job
    a = chess.engine.SimpleEngine.popen_uci([str(BENCH), "uci", args.a])
    b = chess.engine.SimpleEngine.popen_uci([str(BENCH), "uci", args.b])
    try:
        a.configure({"Frames": args.frames})
        b.configure({"Frames": args.frames_b or args.frames})
        board = chess.Board()
        for m in opening.split():
            board.push_uci(m)
        while not board.is_game_over(claim_draw=True) and board.ply() < 300:
            a_turn = (board.turn == chess.WHITE) == a_white
            engine = a if a_turn else b
            result = engine.play(board, chess.engine.Limit(time=3600))
            if result.move is None or result.move not in board.legal_moves:
                loser_is_a = a_turn
                return index, 0.0 if loser_is_a else 1.0, board, f"illegal {result.move}"
            board.push(result.move)
        outcome = board.outcome(claim_draw=True)
        if outcome is None or outcome.winner is None:
            return index, 0.5, board, outcome.termination.name if outcome else "length"
        a_won = (outcome.winner == chess.WHITE) == a_white
        return index, 1.0 if a_won else 0.0, board, outcome.termination.name
    finally:
        a.quit()
        b.quit()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--a", required=True)
    p.add_argument("--b", required=True)
    p.add_argument("--games", type=int, default=64)
    p.add_argument("--frames", type=int, default=600)
    p.add_argument("--frames-b", type=int)
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--seed", type=int, default=7)
    args = p.parse_args()
    openings = OPENINGS[:]
    random.Random(args.seed).shuffle(openings)
    jobs = [(i, openings[(i // 2) % len(openings)], i % 2 == 0, args) for i in range(args.games)]
    total = 0.0
    done = 0
    with concurrent.futures.ProcessPoolExecutor(args.jobs) as pool:
        for index, score, board, why in pool.map(play, jobs):
            total += score
            done += 1
            print(f"game {index:3} A {score:.1f} {why:22} plies {board.ply():3}  running {total}/{done}", flush=True)
    d, lo, hi = elo(total, done)
    print(f"\nA scored {total}/{done}: Elo diff {d:+.0f} (95% {lo:+.0f}..{hi:+.0f})")


if __name__ == "__main__":
    main()
