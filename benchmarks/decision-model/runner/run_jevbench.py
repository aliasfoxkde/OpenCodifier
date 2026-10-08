#!/usr/bin/env python3
"""Run OpenCodifier arms through the OFFICIAL JevBench harness.

Methodology: benchmarks/decision-model/JEVBENCH.md. The harness is the
authors' own package (fstandhartinger/jevbench, MIT), imported from a local
clone — their runner, their scoring (argmax for choice/noul, expected value
for score, multi-class Brier, top-label ECE), their serial no-retry budget
semantics: failed or invalid answers count as incorrect, and no probability
distribution is ever synthesized. Raw evidence lands outside both repos.

Arms (--arm):
  engine      OpenCodifier `serve` (default deterministic stack) over the
              native /v1/decide endpoint, one question per JevBench item:
              choice -> ChoiceQuestion (criteria -> candidate descriptions),
              noul -> BooleanQuestion (p(yes) from the IR answer; the item's
              false:/true: criteria are folded into the question text),
              score -> ScoreQuestion (level descriptions folded into the
              question text — the IR's ScoreLevel carries labels only).
              Probabilities are emitted only where the IR really carries
              them (noul p(yes); score / choice `distribution` when present
              in the native response). Winner-probability-only answers stay
              label-only: splitting the rest-mass evenly would invent
              calibration. An abstaining engine answer is ok=False — their
              runner scores that incorrect, which is the honest outcome.
  fork_4b     The parallel-decision fork's POST /v1/decision (tree mode)
              against an already-running server (--port). Label-only: the
              fork emits winner probability and drops the rest (D15), and
              inventing the tail is exactly what the harness forbids.
  jev_native  Jev-Style-0.8B-Decision-v3 through its native verdict-slot
              readout (see run_jev_native.py) — the comparability bridge
              against the authors' published 64.1% official-harness row.
              noul renders as a two-option choice (no/yes with the item's
              criteria); score renders levels as options; both mappings are
              recorded in the result.
  vtx         VTX-JEV-3 (VTXAI) through its vendor JevClient: the same
              rendering as jev_native (noul as no/yes, score levels as
              options), scored by the model's own cosine softmax. The
              position-gated attention pooler lives in the vendor client —
              plain Model2Vec mean pooling does not reproduce this head.

Every arm replays the whole suite (--replay, default on) and records a
determinism block in the manifest.

Usage:
  python3 runner/run_jevbench.py --arm engine \
      --binary ../../target/release/opencodifier \
      --tasks $REF/datasets/public/easy.jsonl,... \
      --out-dir /nas/Temp/work/oc-model-eval/runs/jevbench/engine

`$REF` is the jevbench clone (commit recorded in the manifest). Results and
raw evidence must live outside both repositories (their Runner enforces it).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path

from resources import ResourceMonitor

RUNNER_DIR = Path(__file__).parent
sys.path.insert(0, str(RUNNER_DIR))


def _load_harness(ref: Path):
    sys.path.insert(0, str(ref))
    from jevbench.runner import Runner  # noqa: PLC0415 (harness import)
    from jevbench.adapters.base import DecisionResult  # noqa: PLC0415
    from jevbench.budget import Ledger  # noqa: PLC0415
    from jevbench.summarize import summarize  # noqa: PLC0415
    from jevbench.tasks import dataset_hash, load_jsonl  # noqa: PLC0415
    return Runner, DecisionResult, Ledger, dataset_hash, load_jsonl, summarize


def harness_commit(ref: Path) -> str:
    out = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ref, capture_output=True, text=True, check=True
    )
    return out.stdout.strip()


def fold_criteria(text: str, criteria) -> str:
    """Render a JevBench rubric into question text for codecs that cannot
    carry it structurally. The item's own content, never invented."""
    if not criteria:
        return text
    if isinstance(criteria, dict):
        parts = [f"{k}: {v}" for k, v in criteria.items()]
    else:  # ordered list (score levels, in labels order)
        parts = [f"{i}: {d}" for i, d in enumerate(criteria)]
    return f"{text} Rubric — " + "; ".join(parts) + "."


def choice_candidates(t) -> list[dict]:
    """labels + criteria -> native candidates. Criteria-less items (92 of
    231) carry the label as its own description — a rendering choice, and
    the raw record says so."""
    criteria = t.question.get("criteria")
    if isinstance(criteria, dict):
        return [{"id": lab, "description": criteria.get(lab, lab)} for lab in t.labels]
    if isinstance(criteria, list):
        return [
            {"id": lab, "description": criteria[i] if i < len(criteria) else lab}
            for i, lab in enumerate(t.labels)
        ]
    return [{"id": lab, "description": lab} for lab in t.labels]


def probs_from_distribution(dist, labels: list[str]):
    """Native `distribution` -> probs over the exact label set, or None when
    it does not cover the labels (never renormalize a partial dict). The
    native serde shape is `{"entries": [{"key": k, "probability": p}]}`."""
    if isinstance(dist, dict) and isinstance(dist.get("entries"), list):
        dist = {
            e["key"]: e["probability"]
            for e in dist["entries"]
            if isinstance(e, dict) and "key" in e and "probability" in e
        }
    if not isinstance(dist, dict):
        return None
    if set(dist) != set(labels):
        return None
    probs = {lab: float(dist[lab]) for lab in labels}
    total = sum(probs.values())
    if not 0.99 <= total <= 1.01:
        return None
    return probs


class EngineAdapter:
    name = "opencodifier-engine"
    price_input_per_m = 0.0
    price_output_per_m = 0.0

    def __init__(
        self,
        binary: Path,
        port: int,
        timeout_s: float,
        log_path: Path | None = None,
        ladder: Path | None = None,
        llama: str | None = None,
        llama_model_id: str | None = None,
        llama_timeout_ms: int | None = None,
    ):
        self.binary = binary
        self.port = port
        self.timeout_s = timeout_s
        self.log_path = log_path
        # Ladder / model-rung wiring mirrors run_engine.py's flags: the
        # request policy the adapter sends is overridden per kind by the
        # ladder's `per_kind` gates (D25), so a fusion ladder and a plain
        # engine arm share one payload contract.
        self.ladder = ladder.resolve() if ladder is not None else None
        self.llama = llama
        self.llama_model_id = llama_model_id
        self.llama_timeout_ms = llama_timeout_ms
        self.proc = None
        self.warnings: list[str] = []

    def load(self):
        from run_engine import wait_health  # noqa: PLC0415 (shared, tested)

        # Server output is evidence: if the serve process dies mid-run the
        # harness stop rule fires and the reason must be on disk.
        sink = open(self.log_path, "wb") if self.log_path else subprocess.DEVNULL
        cmd = [str(self.binary), "serve", "--bind", f"127.0.0.1:{self.port}"]
        if self.ladder is not None:
            # Absolute path: the serve child's cwd is wherever the runner
            # was launched, and a relative ladder silently resolves there.
            cmd += ["--ladder", str(self.ladder)]
        if self.llama is not None:
            cmd += ["--llama", self.llama, "--llama-model-id", self.llama_model_id]
            if self.llama_timeout_ms is not None:
                cmd += ["--llama-timeout-ms", str(self.llama_timeout_ms)]
        self.proc = subprocess.Popen(
            cmd,
            stdout=sink,
            stderr=subprocess.STDOUT,
        )
        wait_health(self.port, self.proc, deadline_s=60.0)

    def reserve_estimate(self, _t):
        return None

    def _post(self, payload: dict):
        import urllib.request  # noqa: PLC0415

        req = urllib.request.Request(
            f"http://127.0.0.1:{self.port}/v1/decide",
            data=json.dumps(payload).encode(),
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=self.timeout_s) as r:
            return json.loads(r.read())

    def run(self, t) -> "DecisionResult":
        from jevbench.adapters.base import DecisionResult  # noqa: PLC0415

        qtype = t.question["type"]
        started = time.perf_counter()
        try:
            if qtype == "choice":
                question = {
                    "type": "choice",
                    "id": "decision",
                    "text": t.question["instructions"],
                    "candidates": choice_candidates(t),
                }
            elif qtype == "noul":
                question = {
                    "type": "boolean",
                    "id": "decision",
                    "text": fold_criteria(t.question["instructions"], t.question.get("criteria")),
                }
            elif qtype == "score":
                question = {
                    "type": "score",
                    "id": "decision",
                    "text": fold_criteria(t.question["instructions"], t.question.get("criteria")),
                    # Native ScoreLevel is a struct; the JevBench labels are
                    # the level labels, descriptions ride in the rubric text.
                    "levels": [{"label": lab} for lab in t.labels],
                }
            else:
                # Structural per-item refusal, same 422 bucket as abstain.
                return DecisionResult(self.name, False, status=422,
                                      error=f"unhandled_type:{qtype}")
            resp = self._post(
                {
                    # Native state is strict: facts must be present, even empty.
                    "state": {
                        "text": t.state if isinstance(t.state, str) else json.dumps(t.state),
                        "facts": {},
                    },
                    "questions": [question],
                    "policy": {
                        "min_confidence": 0.8,
                        "verify_below": 0.65,
                        "abstain_below": 0.5,
                        "risk": "low",
                    },
                    "metadata": {
                        "request_id": f"jevbench-{t.id}",
                        "limits": {
                            "max_input_bytes": 1048576,
                            "max_questions": 32,
                            "max_candidates": 256,
                            "max_graph_nodes": 128,
                            # The adapter's own deadline, not the harness's:
                            # a 15 KB long-policy state over a 5-candidate
                            # tree prefill takes >20 s on the rung, and a
                            # 10 s limit turned that into engine.timeout
                            # 500s (three in a row trips their stop rule).
                            "max_execution_time": {"secs": 120, "nanos": 0},
                            "max_retrieval_results": 64,
                        },
                    },
                }
            )
        except Exception as e:  # noqa: BLE001 — their Runner classifies failures
            return DecisionResult(self.name, False, error=type(e).__name__)
        latency = time.perf_counter() - started
        ans = resp["answers"][0]
        outcome = resp.get("outcome")
        # Trace + metrics are response-level: which rung decided is the
        # routing evidence a fusion arm exists to produce. The native
        # projection ships both verbatim (trace_version + node entries).
        raw = {
            "request": question,
            "answer": ans,
            "outcome": outcome,
            "trace": resp.get("trace"),
            "metrics": resp.get("metrics"),
        }
        if outcome not in ("accept", "verify"):
            # Abstention is a successful engine outcome and an incorrect
            # JevBench answer; no guess is manufactured to dodge it. Status
            # 422 is the runner's "system refused this input" bucket (their
            # runner.py: a 422 counts wrong but is not an outage, so it does
            # not feed the 3-consecutive-infra-errors stop rule) — an
            # abstain is a per-item policy refusal, not an infrastructure
            # failure, and the engine's abstain pattern is deterministic
            # (runs v1 and v2 both stopped at 185/231 on 3 consecutive
            # abstains before this classification).
            return DecisionResult(
                self.name, False, status=422, error=f"engine_{outcome}",
                latency_s=latency, raw=raw,
            )
        probs = None
        if qtype == "noul":
            value = ans["value"]
            # The IR's Boolean answer carries the confidence of the DECIDED
            # value (verified in raw evidence: value=false, probability=0.64
            # means p(no)=0.64), so p(yes) flips for a false answer. An
            # exact 0.5 is the engine's tie — either value is consistent.
            p = float(ans["probability"])
            p_yes = p if value is True else 1.0 - p
            tie = abs(p_yes - 0.5) < 1e-12
            consistent = tie or ((p_yes > 0.5) is (value is True))
            if not 0.0 <= p_yes <= 1.0 or not consistent:
                self.warnings.append(f"{t.id}: value/probability disagree")
                return DecisionResult(
                    self.name, True, label="yes" if value else "no",
                    probs_source="label_only_no_calibrated_distribution",
                    latency_s=latency, raw=raw,
                )
            probs = {"no": 1.0 - p_yes, "yes": p_yes}
            label = "yes" if value else "no"
        elif qtype == "score":
            probs = probs_from_distribution(ans.get("distribution"), t.labels)
            if probs is None:
                return DecisionResult(self.name, False, error="no_score_distribution",
                                      latency_s=latency, raw=raw)
            label = max(probs, key=probs.get)
        else:
            probs = probs_from_distribution(ans.get("distribution"), t.labels)
            label = ans["choice"]
        if probs is not None:
            return DecisionResult(self.name, True, probs=probs, probs_source="native",
                                  label=label, latency_s=latency, raw=raw)
        # Winner-probability-only: the honest choice is label-only, not an
        # invented split of the rest mass.
        return DecisionResult(self.name, True, label=label,
                              probs_source="label_only_no_calibrated_distribution",
                              latency_s=latency, raw=raw)

    def close(self):
        if self.proc is not None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()


class ForkAdapter:
    """Parallel-decision fork, tree mode, against an already-running server.
    Label-only: the fork emits winner probability and drops the rest (D15)."""

    name = "pd-fork-tree"
    price_input_per_m = 0.0
    price_output_per_m = 0.0

    def __init__(self, port: int, instructions: str, timeout_s: float):
        self.port = port
        self.instructions = instructions
        self.timeout_s = timeout_s

    def load(self):
        from run_llama import wait_health  # noqa: PLC0415

        import urllib.request  # noqa: PLC0415

        with urllib.request.urlopen(f"http://127.0.0.1:{self.port}/health", timeout=5):
            pass

    def reserve_estimate(self, _t):
        return None

    def run(self, t) -> "DecisionResult":
        from jevbench.adapters.base import DecisionResult  # noqa: PLC0415
        from run_llama import post  # noqa: PLC0415 (shared, tested)

        qtype = t.question["type"]
        choices = list(t.labels)
        description = t.question["instructions"]
        if qtype == "noul":
            description = fold_criteria(description, t.question.get("criteria"))
        elif qtype == "score":
            description = fold_criteria(description, t.question.get("criteria"))
        # Template A/B sentinel (#76): "@task" repeats the item's own
        # instructions at the top level — the schema description already
        # carries them, this arm measures the emphasis.
        instructions = description if self.instructions == "@task" else self.instructions
        payload = {
            "instructions": instructions,
            "schema": {"choice": {"type": "enum", "choices": choices, "description": description}},
            "contexts": [t.state if isinstance(t.state, str) else json.dumps(t.state)],
            "mode": "tree",
        }
        started = time.perf_counter()
        try:
            resp = post(f"http://127.0.0.1:{self.port}/v1/decision", payload, self.timeout_s)
        except Exception as e:  # noqa: BLE001
            return DecisionResult(self.name, False, error=type(e).__name__)
        latency = time.perf_counter() - started
        field = resp["results"][0]["fields"]["choice"]
        label = field["value"]
        raw = {"request": payload, "field": field}
        if label not in choices:
            return DecisionResult(self.name, False, error="out_of_set_label",
                                  latency_s=latency, raw=raw)
        # The d15 fork adds the full per-choice distribution (additive JSON;
        # pre-d15 binaries omit it and stay label-only, D15).
        probs = probs_from_distribution(field.get("distribution"), choices)
        if probs is not None:
            return DecisionResult(self.name, True, label=label, probs=probs,
                                  probs_source="native",
                                  latency_s=latency, raw=raw)
        return DecisionResult(self.name, True, label=label,
                              probs_source="label_only_no_calibrated_distribution",
                              latency_s=latency, raw=raw)

    def close(self):
        """Runs against an already-running server; owns no process or engine.

        main()'s finally calls this unconditionally, so it must exist even
        though there is nothing to release (the fork4b run of 2026-10-01
        reached 230/231 and died here at teardown on the missing method).
        """


class JevNativeBridgeAdapter:
    """Jev-Style native verdict-slot readout — the comparability bridge.
    Full softmax over options, so Brier/ECE are comparable with the
    published 64.1% row."""

    name = "jev-style-0.8b-native"
    price_input_per_m = 0.0
    price_output_per_m = 0.0

    def __init__(self, model_dir: Path, threads: int, timeout_s: float):
        self.model_dir = model_dir
        self.threads = threads
        self.timeout_s = timeout_s
        self.engine = None

    def load(self):
        from run_jev_native import load_runtime  # noqa: PLC0415 (shared)

        self.engine = load_runtime(self.model_dir, self.threads)

    def reserve_estimate(self, _t):
        return None

    def run(self, t) -> "DecisionResult":
        from jevbench.adapters.base import DecisionResult  # noqa: PLC0415

        qtype = t.question["type"]
        criteria = t.question.get("criteria")
        if qtype == "noul":
            c = criteria if isinstance(criteria, dict) else {}
            options = {"no": c.get("false", "not held"), "yes": c.get("true", "held")}
        elif qtype == "score":
            descs = criteria if isinstance(criteria, list) else [lab for lab in t.labels]
            options = {
                lab: (descs[i] if isinstance(descs, list) and i < len(descs) else lab)
                for i, lab in enumerate(t.labels)
            }
        else:
            options = {c["id"]: c["description"] for c in choice_candidates(t)}
        started = time.perf_counter()
        try:
            res = self.engine.decide(
                t.state if isinstance(t.state, str) else json.dumps(t.state),
                t.question["instructions"],
                options=options,
                category=None,
            )
        except Exception as e:  # noqa: BLE001
            return DecisionResult(self.name, False, error=type(e).__name__)
        latency = time.perf_counter() - started
        probs = {lab: float(res["probabilities"][lab]) for lab in t.labels}
        label = res["answer"]
        raw = {"options": options, "probabilities": res["probabilities"]}
        if label not in t.labels or abs(sum(probs.values()) - 1.0) > 1e-6:
            return DecisionResult(self.name, False, error="invalid_distribution",
                                  latency_s=latency, raw=raw)
        return DecisionResult(self.name, True, probs=probs, probs_source="native",
                              label=label, latency_s=latency, raw=raw)

    def close(self):
        if self.engine is not None:
            self.engine.close()


class VtxAdapter:
    """VTX-JEV-3 static-embedding decision engine through its vendor client.

    All three JevBench kinds render as candidate-conditioned choices over
    the option descriptions — exactly the jev_native bridge's rendering, so
    the two rows are comparable. The model returns a full distribution over
    the option set (cosine softmax, scale 15), so Brier/ECE are comparable
    with the published rows too."""

    name = "vtx-jev-3"
    price_input_per_m = 0.0
    price_output_per_m = 0.0

    def __init__(self, model_dir: Path, timeout_s: float):
        self.model_dir = model_dir
        self.timeout_s = timeout_s
        self.client = None

    def load(self):
        sys.path.insert(0, str(self.model_dir))
        from inference import JevClient  # noqa: PLC0415 (vendor client)

        self.client = JevClient.from_pretrained(str(self.model_dir))

    def reserve_estimate(self, _t):
        return None

    def run(self, t) -> "DecisionResult":
        from inference import Choice  # noqa: PLC0415 (vendor client)
        from jevbench.adapters.base import DecisionResult  # noqa: PLC0415

        qtype = t.question["type"]
        criteria = t.question.get("criteria")
        if qtype == "noul":
            c = criteria if isinstance(criteria, dict) else {}
            options = {"no": c.get("false", "not held"), "yes": c.get("true", "held")}
        elif qtype == "score":
            descs = criteria if isinstance(criteria, list) else [lab for lab in t.labels]
            options = {
                lab: (descs[i] if isinstance(descs, list) and i < len(descs) else lab)
                for i, lab in enumerate(t.labels)
            }
        else:
            options = {c["id"]: c["description"] for c in choice_candidates(t)}
        started = time.perf_counter()
        try:
            response = self.client.system_one(
                state=t.state if isinstance(t.state, str) else json.dumps(t.state),
                questions={"decision": Choice(t.question["instructions"], options)},
            )
        except Exception as e:  # noqa: BLE001 — their Runner classifies failures
            return DecisionResult(self.name, False, error=type(e).__name__)
        latency = time.perf_counter() - started
        result = response.choices.get("decision")
        if result is None:
            return DecisionResult(self.name, False, error="no_decision",
                                  latency_s=latency, raw={"qtype": qtype})
        raw = {"options": options, "distribution": result.distribution}
        probs = probs_from_distribution(dict(result.distribution), t.labels)
        if probs is None or result.choice not in t.labels:
            return DecisionResult(self.name, False, error="invalid_distribution",
                                  latency_s=latency, raw=raw)
        return DecisionResult(self.name, True, probs=probs, probs_source="native",
                              label=result.choice, latency_s=latency, raw=raw)

    def close(self):
        self.client = None


class NliOnnxAdapter:
    """Zero-shot NLI cross-encoder through ONNX Runtime (PLAN 24j; arm of
    record `MoritzLaurer/deberta-v3-base-zeroshot-v2.0`, its own published
    `onnx/model.onnx` — no conversion). All three JevBench kinds render as
    candidate verbalizations over the same option descriptions the
    vtx/jev_native rows use, so the rows are comparable: the premise is
    the item state plus the question instructions, each option's
    description is the hypothesis, and the entailment masses renormalized
    over the option set are the distribution. The shipped serving path is
    the Rust contract (`crates/opencodifier-model/src/nli.rs`); this
    adapter is the benchmark leg of the same arm, mirroring the
    contract's renormalization (zero total mass is refused, never
    uniformed) and D7 (probabilities from graph logits only)."""

    name = "nli-zeroshot"
    price_input_per_m = 0.0
    price_output_per_m = 0.0
    # Engine-only serve metadata; its absence broke the manifest block on
    # every non-engine adapter until this was made explicit.
    ladder = None

    def __init__(self, model_dir: Path, threads: int, timeout_s: float):
        self.model_dir = model_dir
        self.threads = threads
        self.timeout_s = timeout_s
        self.session = None
        self.tok = None
        self.np = None
        self.entail_index = 0

    def load(self):
        import onnxruntime as ort
        from tokenizers import Tokenizer

        self.np = __import__("numpy")
        config = json.loads((self.model_dir / "config.json").read_text())
        id2label = config["id2label"]
        for index in range(len(id2label)):
            if str(id2label[str(index)]).lower() == "entailment":
                self.entail_index = index
                break
        self.tok = Tokenizer.from_file(str(self.model_dir / "tokenizer.json"))
        self.tok.enable_truncation(max_length=512)
        so = ort.SessionOptions()
        so.intra_op_num_threads = self.threads
        self.session = ort.InferenceSession(
            str(self.model_dir / "onnx" / "model.onnx"),
            so,
            providers=["CPUExecutionProvider"],
        )

    def reserve_estimate(self, _t):
        return None

    def _entailment(self, premise: str, hypothesis: str) -> float:
        enc = self.tok.encode(premise, hypothesis)
        feed = {
            "input_ids": self.np.array([enc.ids], dtype=self.np.int64),
            "attention_mask": self.np.array([enc.attention_mask], dtype=self.np.int64),
        }
        (logits,) = self.session.run(None, feed)
        shifted = logits[0] - logits[0].max()
        exp = self.np.exp(shifted)
        return float(exp[self.entail_index] / exp.sum())

    def run(self, t) -> "DecisionResult":
        from jevbench.adapters.base import DecisionResult  # noqa: PLC0415

        qtype = t.question["type"]
        criteria = t.question.get("criteria")
        if qtype == "noul":
            c = criteria if isinstance(criteria, dict) else {}
            options = {"no": c.get("false", "not held"), "yes": c.get("true", "held")}
        elif qtype == "score":
            descs = criteria if isinstance(criteria, list) else [lab for lab in t.labels]
            options = {
                lab: (descs[i] if isinstance(descs, list) and i < len(descs) else lab)
                for i, lab in enumerate(t.labels)
            }
        else:
            options = {c["id"]: c["description"] for c in choice_candidates(t)}
        started = time.perf_counter()
        try:
            premise = (
                (t.state if isinstance(t.state, str) else json.dumps(t.state))
                + "\nQuestion: "
                + t.question["instructions"]
            )
            masses = {
                lab: self._entailment(premise, f"{desc}.") for lab, desc in options.items()
            }
        except Exception as e:  # noqa: BLE001 — their Runner classifies failures
            return DecisionResult(self.name, False, error=f"{type(e).__name__}: {e}"[:200])
        latency = time.perf_counter() - started
        total = sum(masses.values())
        probs = {lab: mass / total for lab, mass in masses.items()} if total > 0 else None
        label = max(probs, key=probs.get) if probs else None
        raw = {"options": options, "entailment_masses": masses, "premise": premise}
        if probs is None or label not in t.labels or abs(sum(probs.values()) - 1.0) > 1e-6:
            return DecisionResult(self.name, False, error="invalid_distribution",
                                  latency_s=latency, raw=raw)
        return DecisionResult(self.name, True, probs=probs, probs_source="native",
                              label=label, latency_s=latency, raw=raw)

    def close(self):
        self.session = None
        self.tok = None


def replay_records(adapter, tasks, ref_dir: Path, out_dir: Path, runner_cls, ledger_cls):
    """Second full pass for the determinism block (fresh ledger + raw dir)."""
    replay_dir = out_dir / "replay"
    replay_dir.mkdir(parents=True, exist_ok=True)
    runner = runner_cls(adapter, ledger_cls(str(replay_dir / "ledger.jsonl"), cap_usd=0.0),
                        raw_dir=replay_dir / "raw", default_reserve_usd=0.0)
    return runner.run_all(tasks)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--arm", required=True,
                    choices=["engine", "fork_4b", "jev_native", "vtx", "nli"])
    ap.add_argument("--ref", type=Path, default=Path("/nas/Temp/work/oc-model-eval/jevbench-ref"))
    ap.add_argument("--tasks", type=str, required=True, help="comma-separated jsonl files")
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument(
        "--binary", type=Path, default=RUNNER_DIR.parents[2] / "target/release/opencodifier"
    )
    ap.add_argument("--port", type=int, default=8178)
    ap.add_argument("--instructions", type=str, default="Select the correct option.")
    ap.add_argument(
        "--model-dir", type=Path, default=Path("/nas/Temp/work/oc-model-eval/jev-v3-native")
    )
    ap.add_argument(
        "--vtx-dir",
        type=Path,
        default=Path("/nas/Temp/work/oc-model-eval/models/vtx-jev-3"),
        help="local VTXAI/VTX-JEV-3 checkout (vtx arm)",
    )
    ap.add_argument(
        "--nli-dir",
        type=Path,
        default=Path("/nas/Temp/work/oc-model-eval/models/deberta-v3-base-zeroshot-v2.0"),
        help="local zero-shot NLI model dir with onnx/model.onnx (nli arm)",
    )
    ap.add_argument("--threads", type=int, default=12)
    ap.add_argument("--timeout", type=float, default=600.0)
    ap.add_argument("--no-replay", action="store_true")
    ap.add_argument("--limit", type=int, default=None)
    ap.add_argument("--ladder", type=Path, default=None, help="pass --ladder to `serve` (engine arm)")
    ap.add_argument(
        "--llama",
        default=None,
        help="pass --llama to `serve` (model rung URL; engine arm)",
    )
    ap.add_argument(
        "--llama-model-id",
        default=None,
        help="pass --llama-model-id to `serve` (required with --llama)",
    )
    ap.add_argument(
        "--llama-timeout-ms",
        type=int,
        default=None,
        help="pass --llama-timeout-ms to `serve`",
    )
    args = ap.parse_args()
    if args.llama is not None and args.llama_model_id is None:
        ap.error("--llama requires --llama-model-id")
    if (args.ladder is not None or args.llama is not None) and args.arm != "engine":
        ap.error("--ladder/--llama only apply to the engine arm")

    Runner, DecisionResult, Ledger, dataset_hash, load_jsonl, summarize = _load_harness(args.ref)

    tasks = []
    for part in args.tasks.split(","):
        if part.strip():
            tasks.extend(load_jsonl(part.strip()))
    if args.limit:
        tasks = tasks[: args.limit]

    if args.arm == "engine":
        adapter = EngineAdapter(
            args.binary,
            args.port,
            args.timeout,
            log_path=args.out_dir / "server.log",
            ladder=args.ladder,
            llama=args.llama,
            llama_model_id=args.llama_model_id,
            llama_timeout_ms=args.llama_timeout_ms,
        )
    elif args.arm == "fork_4b":
        adapter = ForkAdapter(args.port, args.instructions, args.timeout)
    elif args.arm == "vtx":
        adapter = VtxAdapter(args.vtx_dir, args.timeout)
    elif args.arm == "nli":
        adapter = NliOnnxAdapter(args.nli_dir, args.threads, args.timeout)
    else:
        adapter = JevNativeBridgeAdapter(args.model_dir, args.threads, args.timeout)

    args.out_dir.mkdir(parents=True, exist_ok=True)
    # Rooted here so every adapter shape is covered: engine/fork adapters
    # spawn a server child; vtx/jev_native load weights in-process.
    monitor = ResourceMonitor()
    monitor.start()
    raw_dir = (args.out_dir / "raw").resolve()
    ledger = Ledger(str(args.out_dir / "ledger.jsonl"), cap_usd=0.0)
    # Every arm is local compute with zero tariff: nothing is billable, so
    # the reserve is $0 rather than a pretend budget the run could "exceed".
    runner = Runner(adapter, ledger, raw_dir=raw_dir, default_reserve_usd=0.0)

    if hasattr(adapter, "load"):
        t0 = time.perf_counter()
        adapter.load()
        print(f"[jevbench] warm load {time.perf_counter() - t0:.1f}s", flush=True)

    try:
        records = runner.run_all(tasks, results_path=args.out_dir / "results.jsonl")
        replay = None
        if not args.no_replay:
            replay = replay_records(adapter, tasks, args.ref, args.out_dir, Runner, Ledger)
    finally:
        adapter.close()

    summary = summarize(tasks, records, ledger_charged=ledger.charged, headline_only=True)
    (args.out_dir / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")

    first = {r["task_id"]: r for r in records}
    determinism = None
    if replay is not None:
        first_by_id = {r["task_id"]: r for r in replay}
        same = sum(
            1
            for tid, r in first_by_id.items()
            if tid in first and r["correct"] == first[tid]["correct"]
        )
        determinism = {
            "attempted": len(replay),
            "correct_match": same,
            "predictions_match": same == len(records) == len(replay),
        }
        (args.out_dir / "results-replay.jsonl").write_text(
            "".join(json.dumps(r, allow_nan=False) + "\n" for r in replay))

    monitor.stop()
    model_files = {}
    if args.arm == "jev_native":
        gguf = args.model_dir / "Jev-Style-0.8B-Decision-v3-Q4_K_M.gguf"
        if gguf.is_file():
            model_files[gguf.name] = hashlib.sha256(gguf.read_bytes()).hexdigest()
    if args.arm == "vtx":
        for fname in ("model_lf2.safetensors", "model.safetensors", "gate_params.npz",
                      "tokenizer.json", "inference.py"):
            f = args.vtx_dir / fname
            if f.is_file():
                model_files[f"vtx/{fname}"] = hashlib.sha256(f.read_bytes()).hexdigest()
    if args.arm == "nli":
        for fname in ("onnx/model.onnx", "tokenizer.json", "config.json"):
            f = args.nli_dir / fname
            if f.is_file():
                model_files[fname] = hashlib.sha256(f.read_bytes()).hexdigest()

    manifest = {
        "arm": args.arm,
        "harness": {"repo": "fstandhartinger/jevbench", "commit": harness_commit(args.ref),
                    "license": "MIT", "method": "docs/METHOD-v1.4.md (public split)"},
        "dataset_sha256": dataset_hash(tasks),
        **(
            {
                "serve": {
                    "ladder": str(adapter.ladder),
                    "ladder_sha256": hashlib.sha256(adapter.ladder.read_bytes()).hexdigest(),
                    "llama": adapter.llama,
                    "llama_model_id": adapter.llama_model_id,
                }
                if adapter.ladder is not None
                else {}
            }
        ),
        "n_tasks": len(tasks),
        "model_files": model_files,
        "mapping_notes": {
            "engine": "choice candidates from labels+criteria; no-criteria items use the "
                      "label as description; noul -> boolean, criteria folded into text; "
                      "score -> score question, level rubric folded into text; "
                      "abstain -> ok=False (incorrect); winner-only -> label-only",
            "fork_4b": "tree mode, label-only (fork drops non-winner mass, D15)",
            "jev_native": "noul as two-option choice; score levels as options; softmax native",
            "vtx": "noul as two-option choice; score levels as options; vendor cosine "
                   "softmax (scale 15) through JevClient's position-gated pooler",
            "nli": "all kinds as option descriptions (same rendering as vtx/jev_native); "
                   "premise = state + '\\nQuestion: ' + instructions, hypothesis = "
                   "'<description>.' (bare template); entailment index from config.json "
                   "id2label; entailment masses renormalized over options; note: the "
                   "Rust serving boolean path uses one-hypothesis complement (IR "
                   "booleans carry only text), this adapter verbalizes per-option from "
                   "JevBench criteria like the zeroshot training objective",
        }[args.arm],
        "determinism": determinism,
        "adapter_warnings": getattr(adapter, "warnings", []),
        "charged_usd": ledger.charged,
        "resources": monitor.report(),
    }
    (args.out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    print(json.dumps({"arm": args.arm, "n": len(records),
                      "summary_keys": sorted(summary.keys())}, sort_keys=True), flush=True)
    return 0 if len(records) == len(tasks) else 3


if __name__ == "__main__":
    sys.exit(main())
