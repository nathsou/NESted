#!/usr/bin/env python3
"""Builds the NESted Chess opening book with Stockfish.

Positions are expanded from the initial position with a probability budget:
Stockfish's best move inherits most of a position's weight, near-equal
alternatives the rest, and positions below a threshold are not expanded.
Each entry is 5 bytes: the engine's 24-bit Zobrist key of the position
(h0, h1, h2) and the move as 0x88 from/to squares. Keys are computed exactly
as the cartridge does, from games/assets/chess.zobrist.bin.

  scripts/chess/book.py --entries 700
"""
import argparse
import pathlib

import chess
import chess.engine

ROOT = pathlib.Path(__file__).resolve().parents[2]
ASSETS = ROOT / "games" / "assets"
Z = (ASSETS / "chess.zobrist.bin").read_bytes()
GROUP_OFFSET = [0, 8, 128, 136]


def sq88(square):
    return chess.square_rank(square) * 16 + chess.square_file(square)


def piece_code(piece):
    return piece.piece_type | (0 if piece.color else 8)


def key(board, ep88):
    """The cartridge's (h0, h1, h2) for `board`, with its own ep rule:
    ep88 is the square behind a pawn that just advanced two squares."""
    h = [0, 0, 0]
    for square, piece in board.piece_map().items():
        p = piece_code(piece)
        group = (p & 7) - 1 + (6 if p & 8 else 0)
        table, offset = group // 4, GROUP_OFFSET[group % 4]
        i = sq88(square) | offset
        for b in range(3):
            h[b] ^= Z[(table * 3 + b) * 256 + i]
    castle = 0
    if board.has_kingside_castling_rights(chess.WHITE):
        castle |= 1
    if board.has_queenside_castling_rights(chess.WHITE):
        castle |= 2
    if board.has_kingside_castling_rights(chess.BLACK):
        castle |= 4
    if board.has_queenside_castling_rights(chess.BLACK):
        castle |= 8
    for b in range(3):
        h[b] ^= Z[2304 + b * 16 + castle]
    if ep88 is not None:
        for b in range(3):
            h[b] ^= Z[2352 + b * 8 + (ep88 & 7)]
    if board.turn == chess.BLACK:
        for b in range(3):
            h[b] ^= Z[2376 + b]
    return tuple(h)


def ep_after(board, move):
    piece = board.piece_at(move.from_square)
    if piece and piece.piece_type == chess.PAWN and abs(move.to_square - move.from_square) == 16:
        return sq88((move.from_square + move.to_square) // 2)
    return None


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--entries", type=int, default=700)
    a.add_argument("--depth", type=int, default=18)
    a.add_argument("--max-ply", type=int, default=14)
    a.add_argument("--margin", type=int, default=30, help="centipawns from best")
    a.add_argument("--stockfish", default="/usr/games/stockfish")
    args = a.parse_args()
    engine = chess.engine.SimpleEngine.popen_uci(args.stockfish)
    engine.configure({"Threads": 4, "Hash": 256})
    # Priority expansion: most probable positions first.
    frontier = [(1.0, chess.Board(), None)]
    entries = []
    seen = set()
    while frontier and len(entries) < args.entries:
        frontier.sort(key=lambda f: -f[0])
        weight, board, ep88 = frontier.pop(0)
        k = key(board, ep88)
        if k in seen or board.ply() >= args.max_ply:
            continue
        seen.add(k)
        infos = engine.analyse(board, chess.engine.Limit(depth=args.depth), multipv=3)
        best = infos[0]["score"].relative.score(mate_score=100000)
        choices = []
        for info in infos:
            score = info["score"].relative.score(mate_score=100000)
            if "pv" in info and best - score <= args.margin:
                choices.append((info["pv"][0], score))
        shares = [0.65, 0.25, 0.10][: len(choices)]
        total = sum(shares)
        for (move, _), share in zip(choices, shares):
            entries.append((k, move))
            child = board.copy()
            child_ep = ep_after(child, move)
            child.push(move)
            frontier.append((weight * share / total, child, child_ep))
    engine.quit()
    data = bytearray()
    for (h0, h1, h2), move in entries:
        data += bytes([h0, h1, h2, sq88(move.from_square), sq88(move.to_square)])
    (ASSETS / "chess.book.bin").write_bytes(bytes(data))
    print(f"{len(entries)} entries, {len(seen)} positions, {len(data)} bytes")


if __name__ == "__main__":
    main()
