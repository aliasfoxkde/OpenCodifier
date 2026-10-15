#!/usr/bin/env python3
"""Fixture tests for run_jevbench.py's slot arm (torch-free, stdlib only).

The slot arm's number is only comparable with the e1 lane if its render
is byte-identical to the SFT training text — the same premise the
teacher_score_rows parity self-test pins for the scoring campaign. These
tests pin that parity at the JevBench boundary: the harness's three
kinds must come out as macjev-render-v1 text (prep-rendered header +
``<option> ->`` slot lines, verdict tokens absent), with harness label
names in slot order — noul's no/yes keyed to the false/true slots — and
the readout math identical to e1_eval's.

Run: python3 -m pytest benchmarks/decision-model/runner/test_run_jevbench_slot.py -q
"""

from __future__ import annotations

import contextlib
import math
import sys
from pathlib import Path
from types import SimpleNamespace

RUNNER = Path(__file__).resolve().parent
sys.path.insert(0, str(RUNNER))

import decision_sft_prep as prep  # noqa: E402
import e1_eval  # noqa: E402
import run_jevbench as rj  # noqa: E402


def make_task(qtype: str, criteria, labels: list[str], instructions: str,
              state="The vendor deadline is Friday.") -> SimpleNamespace:
    return SimpleNamespace(
        state=state,
        question={"type": qtype, "instructions": instructions,
                  "criteria": criteria},
        labels=labels,
    )


def test_choice_render_is_sft_and_suite_identical():
    t = make_task("choice", {"v-a": "ships in 2 days",
                             "v-b": "ships in 5 days"},
                  ["v-a", "v-b"], "Which vendor ships first?")
    text, names, options = rj.render_slot_item(t)
    state = "The vendor deadline is Friday."
    # SFT training parity: prep-rendered header + slot lines + closing \n
    expected = (prep.render_header("choice", "Which vendor ships first?",
                                   options, state)
                + "\n".join(f"{o} ->" for o in options) + "\n")
    assert text == expected
    # Suite-parity: the same item through e1_eval.render_choice
    candidates = [{"id": "v-a", "description": "ships in 2 days"},
                  {"id": "v-b", "description": "ships in 5 days"}]
    suite_text, suite_ids = e1_eval.render_choice(
        state, "Which vendor ships first?", candidates)
    assert text == suite_text
    assert names == suite_ids == ["v-a", "v-b"]


def test_noul_renders_boolean_slots_keyed_no_yes():
    t = make_task("noul", {"true": "the contract was held",
                           "false": "not held"},
                  ["no", "yes"], "Was the clause upheld?")
    text, names, options = rj.render_slot_item(t)
    assert options == ["false: not held", "true: the contract was held"]
    assert names == ["no", "yes"]
    expected = (prep.render_header("noul", "Was the clause upheld?",
                                   options, t.state)
                + "\n".join(f"{o} ->" for o in options) + "\n")
    assert text == expected


def test_noul_without_criteria_renders_bare_slots():
    t = make_task("noul", None, ["no", "yes"], "Was it upheld?")
    _, _, options = rj.render_slot_item(t)
    assert options == ["false:", "true:"]


def test_score_renders_levels_as_options():
    t = make_task("score", ["breach", "partial", "full"],
                  ["0", "1", "2"], "Rate the delivery.")
    text, names, options = rj.render_slot_item(t)
    assert options == ["level 0: breach", "level 1: partial",
                       "level 2: full"]
    assert names == ["0", "1", "2"]
    expected = (prep.render_header("score", "Rate the delivery.", options,
                                   t.state)
                + "\n".join(f"{o} ->" for o in options) + "\n")
    assert text == expected


def test_criterialess_choice_uses_label_as_description():
    t = make_task("choice", None, ["A", "B"], "Pick one.")
    _, names, options = rj.render_slot_item(t)
    assert options == ["A: A", "B: B"]
    assert names == ["A", "B"]


def test_unrenderable_items_return_none():
    assert rj.render_slot_item(
        make_task("essay", None, ["a", "b"], "Write prose.")) is None
    assert rj.render_slot_item(
        make_task("choice", {"v-a": "x", "v-b": "y"},
                  ["v-a", "v-b"], "")) is None  # empty instructions


def test_slot_softmax_matches_e1_readout_math():
    scores = [2.0, 0.5, -1.0]
    probs = rj.slot_softmax(scores)
    mx = max(scores)
    exps = [math.exp(s - mx) for s in scores]
    total = sum(exps)
    assert probs == [e / total for e in exps]
    assert abs(sum(probs) - 1.0) < 1e-12


class _Row:
    def __init__(self, table):
        self._t = table

    def __getitem__(self, key):
        return self._t[key]


class _Logits:
    def __init__(self, table):
        self._t = table

    def __getitem__(self, _i):
        return _Row(self._t)


class FakeModel:
    """Derives the ' ->' anchors from the token ids it is shown (the
    char-level tokenizer makes ids == list(text)) and serves
    per-slot yes-margins from the table built there; " no" logit is 0.
    Records the ids so tests can assert the render reached the model."""

    def __init__(self, scores_by_slot: list[float]):
        self.scores = list(scores_by_slot)
        self.logits = None
        self.last_ids = None

    def __call__(self, ids):
        self.last_ids = ids
        text = "".join(ids)
        positions = e1_eval.slot_positions(text, len(self.scores), list)
        table = {}
        for i, p in enumerate(positions or []):
            table[(p, 1)] = self.scores[i]
            table[(p, 2)] = 0.0
        self.logits = _Logits(table)
        return self


def make_fake_adapter(monkeypatch, scores_by_slot: list[float]):
    """E1SlotAdapter with a char-level fake tokenizer and a fake model
    whose yes-margin per slot is scores_by_slot."""
    ad = rj.E1SlotAdapter(Path("/nonexistent/base"), None, "bf16", 1.0)
    ad.tok = SimpleNamespace(
        encode=lambda s, add_special_tokens=True: list(s))
    ad.model = FakeModel(scores_by_slot)
    ad.device = "cpu"
    ad.yes_id, ad.no_id = 1, 2

    fake_torch = SimpleNamespace(
        # the adapter calls tensor([ids]) — unwrap the batch of one
        tensor=lambda nested, device=None: nested[0],
        no_grad=contextlib.nullcontext,
    )
    monkeypatch.setitem(sys.modules, "torch", fake_torch)
    return ad


def stub_decision_result(monkeypatch, tmp_path):
    pkg = tmp_path / "jevbench" / "adapters"
    pkg.mkdir(parents=True)
    (tmp_path / "jevbench" / "__init__.py").write_text("")
    (pkg / "__init__.py").write_text("")
    (pkg / "base.py").write_text(
        "class DecisionResult:\n"
        "    def __init__(self, name, ok, *, probs=None, probs_source=None,\n"
        "                 label=None, latency_s=None, error=None, raw=None):\n"
        "        self.name, self.ok = name, ok\n"
        "        self.probs, self.probs_source = probs, probs_source\n"
        "        self.label, self.latency_s = label, latency_s\n"
        "        self.error, self.raw = error, raw\n")
    monkeypatch.syspath_prepend(str(tmp_path))
    for mod in ("jevbench", "jevbench.adapters", "jevbench.adapters.base"):
        monkeypatch.delitem(sys.modules, mod, raising=False)


def test_choice_run_scores_slots_and_normalizes(monkeypatch, tmp_path):
    stub_decision_result(monkeypatch, tmp_path)
    ad = make_fake_adapter(monkeypatch, [2.0, 0.5, -1.0])
    t = make_task("choice", {"v-a": "ships in 2 days",
                             "v-b": "ships in 5 days",
                             "v-c": "never ships"},
                  ["v-a", "v-b", "v-c"], "Which vendor ships first?")
    res = ad.run(t)
    assert res.ok and res.error is None
    assert res.probs_source == "native"
    assert res.label == "v-a"
    assert abs(sum(res.probs.values()) - 1.0) < 1e-9
    assert list(res.probs) == ["v-a", "v-b", "v-c"]
    assert res.probs == dict(zip(["v-a", "v-b", "v-c"],
                                 rj.slot_softmax([2.0, 0.5, -1.0])))
    assert len(res.raw["slot_scores"]) == 3
    assert res.latency_s is not None


def test_noul_false_slot_maps_to_no_label(monkeypatch, tmp_path):
    stub_decision_result(monkeypatch, tmp_path)
    # false slot (position 0) wins -> the harness label must be 'no'
    ad = make_fake_adapter(monkeypatch, [1.5, -0.5])
    t = make_task("noul", {"true": "held", "false": "not held"},
                  ["no", "yes"], "Was it upheld?")
    res = ad.run(t)
    assert res.ok and res.label == "no"
    assert list(res.probs) == ["no", "yes"]
    # and the mirror: true slot wins -> 'yes'
    ad2 = make_fake_adapter(monkeypatch, [-0.5, 1.5])
    res2 = ad2.run(t)
    assert res2.ok and res2.label == "yes"


def test_score_run_keys_probs_by_harness_labels(monkeypatch, tmp_path):
    stub_decision_result(monkeypatch, tmp_path)
    ad = make_fake_adapter(monkeypatch, [-1.0, 0.25, 0.9])
    t = make_task("score", ["breach", "partial", "full"],
                  ["0", "1", "2"], "Rate the delivery.")
    res = ad.run(t)
    assert res.ok and res.label == "2"
    assert list(res.probs) == ["0", "1", "2"]


def test_run_rejects_unrenderable_question(monkeypatch, tmp_path):
    stub_decision_result(monkeypatch, tmp_path)
    ad = make_fake_adapter(monkeypatch, [1.0, 0.0])
    res = ad.run(make_task("essay", None, ["a", "b"], "Write."))
    assert not res.ok and res.error == "unrenderable_question"
