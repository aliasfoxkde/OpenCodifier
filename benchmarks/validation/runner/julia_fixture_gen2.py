#!/usr/bin/env python3
"""Rebuild the Julia-1 fixture tokenizer as a derivation closure that is
VERIFIED against a faithful re-implementation of the Rust oracle.

Why not just keep the final ids: the Rust BpeTokenizer re-derives ids by
byte-level BPE (GPT-2 byte alphabet, whole segment, no pre-tokenization),
so reproduction needs every intermediate merge step, not the endpoints.

Why not mirror real merge ranks: best-first BPE can converge to a
different fixed point than the real per-pretoken tokenization. Instead:

  1. Each real token maps to its text span's bytes, spelled through the
     GPT-2 alphabet; the fixture vocab assigns that spelling the REAL id.
  2. A byte->spelling merge CHAIN composes the spelling; chains are
     ordered longest-token-first and precede everything, so inside a
     run the run's own chain always outranks any shorter interfering
     chain.
  3. The oracle algorithm is re-implemented here and EVERY frozen
     segment is checked end-to-end (ids AND the marker-bearing split)
     before the tokenizer is written; any residual divergence is a hard
     failure, not a shrug.
"""
import json

from tokenizers import Tokenizer

BASE = "/home/mkinney/oc-model-eval/models/onnx/julia-1"
OUT_TOKENIZER = "/nas/Temp/tmp/julia-fixture-tokenizer.json"
OUT_PROBE = "/nas/Temp/tmp/julia-fixture-probe.json"


def gpt2_alphabet() -> list[str]:
    """The exact mapping `byte_alphabet()` in kai.rs uses."""
    alphabet = ["\0"] * 256
    nxt = 256
    for byte in range(256):
        if 33 <= byte <= 126 or 161 <= byte <= 172 or 174 <= byte <= 255:
            alphabet[byte] = chr(byte)
        else:
            alphabet[byte] = chr(nxt)
            nxt += 1
    return alphabet


ALPHABET = gpt2_alphabet()


def spell(raw: bytes) -> str:
    return "".join(ALPHABET[b] for b in raw)


def oracle_symbols(text: str, ranks: dict, log: list | None = None) -> list[str]:
    """kai.rs `BpeTokenizer::encode`, returning the final symbols."""
    symbols = [ALPHABET[b] for b in text.encode("utf-8")]
    widths = [1] * len(symbols)  # byte width of each symbol
    while True:
        best = None
        for index in range(len(symbols) - 1):
            rank = ranks.get((symbols[index], symbols[index + 1]))
            if rank is not None and (best is None or rank < best[0]):
                best = (rank, index)
        if best is None:
            break
        _, index = best
        if log is not None:
            start = sum(widths[:index])
            log.append((symbols[index], symbols[index + 1], start, widths[index] + widths[index + 1]))
        merged = symbols[index] + symbols[index + 1]
        width = widths[index] + widths[index + 1]
        symbols[index:index + 2] = [merged]
        widths[index:index + 2] = [width]
    return symbols


def oracle_encode(text: str, vocab: dict[str, int], ranks: dict) -> list[int]:
    try:
        return [vocab[symbol] for symbol in oracle_symbols(text, ranks)]
    except KeyError as missing:
        raise SystemExit(f"oracle produced unknown symbol {missing}") from missing


tok = Tokenizer.from_file(f"{BASE}/tokenizer.json")
fixtures = json.load(open("/home/mkinney/oc-model-eval/julia-reference-trimmed.json"))

segments = []  # (label, text, expected ids)
for case in fixtures["cases"]:
    head = tok.encode(case["head_text"], add_special_tokens=False).ids
    segments.append((case["id"] + ":head", case["head_text"], head))
    for index, option in enumerate(case["option_texts"]):
        text = " " + option
        segments.append(
            (f"{case['id']}:opt{index}", text,
             tok.encode(text, add_special_tokens=False).ids))
    if case["state_text"]:
        text = case["state_text"]
        segments.append(
            (case["id"] + ":state", text, tok.encode(text, add_special_tokens=False).ids))

for label, text, _ in segments:
    if not text.isascii():
        raise SystemExit(f"non-ASCII segment {label!r}: {text!r}")

# real token spellings (oracle spelling -> real id), longest first
spellings: dict[str, int] = {}
for label, text, ids in segments:
    raw = text.encode("utf-8")
    encoding = tok.encode(text, add_special_tokens=False)
    for token_id, (start, end) in zip(encoding.ids, encoding.offsets):
        key = spell(raw[start:end])
        if spellings.setdefault(key, token_id) != token_id:
            raise SystemExit(f"{label}: {key!r} maps to two ids")
order = sorted(spellings, key=lambda key: (not key.startswith("Ġ"), -len(key)))

vocab: dict[str, int] = dict(spellings)
merges: list[tuple[str, str]] = []
seen: set[tuple[str, str]] = set()


def emit(left: str, right: str) -> None:
    if (left, right) not in seen:
        seen.add((left, right))
        merges.append((left, right))


for key in order:  # metaspace runs first, then longest: own chain wins races
    acc = key[0]
    for part in key[1:]:
        nxt = acc + part
        if nxt != key:
            vocab.setdefault(nxt, 1_000_000 + len(vocab))
        emit(acc, part)
        acc = nxt
for label, text, _ in segments:
    for byte in text.encode("utf-8"):
        vocab.setdefault(spell(bytes([byte])), 1_000_000 + len(vocab))

chain_of: dict[str, set[tuple[str, str]]] = {}
for key in spellings:
    pairs = set()
    acc = key[0]
    for part in key[1:]:
        pairs.add((acc, part))
        acc = acc + part
    chain_of[key] = pairs


def demote(pair: tuple[str, str]) -> None:
    """Move `pair` to the end of the merge list (loses every race)."""
    merges.remove(pair)
    merges.append(pair)


ranks = {pair: index for index, pair in enumerate(merges)}

# verify every segment end-to-end; demote pairs that fire outside the
# chain of the run they land in, until everything reproduces
for _round in range(60):
    failures = []
    changed = False
    for label, text, expected in segments:
        got = oracle_encode(text, vocab, ranks)
        if got == expected:
            continue
        # replay with logging and find pairs that fired outside any
        # intended run's chain
        spans = []
        raw = text.encode("utf-8")
        encoding = tok.encode(text, add_special_tokens=False)
        for _token_id, (start, end) in zip(encoding.ids, encoding.offsets):
            spans.append((start, end, spell(raw[start:end])))
        log: list = []
        oracle_symbols(text, ranks, log)
        for left, right, start, width in log:
            cover = next(
                (spelling for begin, end, spelling in spans
                 if begin <= start and start + width <= end),
                None)
            if cover is None or (left, right) not in chain_of[cover]:
                demote((left, right))
                changed = True
        failures.append((label, expected, got))
    ranks = {pair: index for index, pair in enumerate(merges)}
    if not failures:
        break
    if not changed:
        for label, expected, got in failures:
            print(f"MISMATCH {label}\n  want {expected}\n  got  {got}")
        raise SystemExit(f"{len(failures)} segment(s) diverge and repair stalled")
else:
    raise SystemExit("repair did not converge in 60 rounds")

# every segment now reproduces byte for byte; write the tokenizer
doc = {
    "version": "1.0",
    "provenance": "derivation closure of julia-reference-trimmed.json segments; "
                  "real ids for real token spellings, filler ids for intermediates; "
                  "metaspace-first chains with demote-repair, verified against the "
                  "Rust oracle algorithm before shipping",
    "model": {
        "type": "BPE",
        "vocab": vocab,
        "merges": [f"{left} {right}" for left, right in merges],
    },
}
with open(OUT_TOKENIZER, "w", encoding="utf-8") as handle:
    json.dump(doc, handle, ensure_ascii=False)

probe = {
    label: {"text": text, "real_ids": ids}
    for label, text, ids in segments
}
with open(OUT_PROBE, "w", encoding="utf-8") as handle:
    json.dump(probe, handle, ensure_ascii=False, indent=1)

print(f"vocab={len(vocab)} merges={len(merges)} segments={len(segments)} all verified")
