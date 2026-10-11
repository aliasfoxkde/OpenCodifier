#!/usr/bin/env python3
"""Fixture tests for run_jevbench.py's minicpm arm (torch-free, stdlib only).

The arm drives ytbai/MiniCPM5-2B-Jev through the authors' own model.py,
so the only contract this repo owns is the wire record: label order must
survive end-to-end (choice criteria rebuilt in labels order, noul to
their fixed no/yes option order, score positional), and the probability
list must map back onto the harness label set without renormalizing a
mismatched slot count. The e2e tests stub their model.py from a fake
checkpoint dir and torch itself, so no real torch/transformers loads.

Run: python3 -m pytest benchmarks/decision-model/runner/test_run_jevbench_minicpm.py -q
"""

from __future__ import annotations

import sys
from pathlib import Path
from types import SimpleNamespace

RUNNER = Path(__file__).resolve().parent
sys.path.insert(0, str(RUNNER))

import run_jevbench as rj  # noqa: E402


def make_task(qtype: str, criteria, labels: list[str], instructions: str,
              state="The vendor deadline is Friday.") -> SimpleNamespace:
    return SimpleNamespace(
        id="t-1",
        state=state,
        question={"type": qtype, "instructions": instructions,
                  "criteria": criteria},
        labels=labels,
    )


def test_choice_wire_keeps_labels_order_and_folds_criteria():
    t = make_task("choice", {"v-b": "ships in 5 days",
                             "v-a": "ships in 2 days"},
                  ["v-a", "v-b"], "Which vendor ships first?")
    wire, names = rj.render_minicpm_wire(t)
    q = wire["questions"]["decision"]
    # criteria rebuilt in LABELS order (v-a first), not dict order
    assert list(q["criteria"]) == ["v-a", "v-b"]
    assert q["criteria"] == {"v-a": "ships in 2 days",
                             "v-b": "ships in 5 days"}
    assert names == ["v-a", "v-b"]
    assert q["type"] == "choice"
    assert wire["state"] == t.state


def test_choice_without_criteria_uses_label_as_description():
    wire, names = rj.render_minicpm_wire(
        make_task("choice", None, ["A", "B"], "Pick one."))
    assert wire["questions"]["decision"]["criteria"] == {"A": "A", "B": "B"}
    assert names == ["A", "B"]


def test_noul_wire_passes_criteria_and_maps_no_yes():
    crit = {"true": "the contract was held", "false": "not held"}
    wire, names = rj.render_minicpm_wire(
        make_task("noul", crit, ["no", "yes"], "Was the clause upheld?"))
    assert wire["questions"]["decision"]["criteria"] == crit
    assert names == ["no", "yes"]


def test_noul_wire_without_criteria_passes_none():
    wire, names = rj.render_minicpm_wire(
        make_task("noul", None, ["no", "yes"], "Was it upheld?"))
    assert wire["questions"]["decision"]["criteria"] is None
    assert names == ["no", "yes"]


def test_score_wire_is_positional_list():
    wire, names = rj.render_minicpm_wire(
        make_task("score", ["breach", "partial", "full"],
                  ["0", "1", "2"], "Rate the delivery."))
    assert wire["questions"]["decision"]["criteria"] == [
        "breach", "partial", "full"]
    assert names == ["0", "1", "2"]


def test_score_wire_dict_degrades_to_labels():
    """Their dict branch sorts by float(key); non-numeric level names
    would crash it, so the dict degrades to the bare labels list."""
    wire, _ = rj.render_minicpm_wire(
        make_task("score", {"lo": "breach", "hi": "full"},
                  ["lo", "hi"], "Rate the delivery."))
    assert wire["questions"]["decision"]["criteria"] == ["lo", "hi"]


def test_unrenderable_items_return_none():
    assert rj.render_minicpm_wire(
        make_task("essay", None, ["a", "b"], "Write prose.")) is None
    assert rj.render_minicpm_wire(
        make_task("choice", {"a": "x", "b": "y"}, ["a", "b"], "")) is None
    assert rj.render_minicpm_wire(
        make_task("choice", {"a": "x"}, ["a"], "Only one.")) is None


STUB_MODEL_PY = '''
"""Stub of ytbai's model.py — records the wire record and returns fixed
probs; loaded by MiniCPMAdapter.load() through importlib."""

LAST_WIRE = {}
LAST_KWARGS = {}


class _Head:
    temperature = 1.0

    def load_state_dict(self, state, strict=False):
        if "bad" in state:
            raise RuntimeError("size mismatch")
        self.loaded = state

    def to(self, device):
        return self


def to_internal_record(rec, add_date_facts=False):
    LAST_WIRE["rec"] = rec
    LAST_WIRE["add_date_facts"] = add_date_facts
    q = rec["questions"]["decision"]
    crit = q["criteria"]
    n = len(crit) if isinstance(crit, (dict, list)) else 2
    return {"id": rec.get("id", ""), "state": rec["state"],
            "questions": [{"name": "decision", "type": q["type"],
                           "options": ["opt-%d" % i for i in range(n)]}]}


def load_tokenizer(name):
    return None


class MiniCPMSystemOne:
    def __init__(self, **kwargs):
        LAST_KWARGS.clear()
        LAST_KWARGS.update(kwargs)
        self.head = _Head()
        self.tok = kwargs.get("tok")

    @classmethod
    def load_checkpoint(cls, ckpt_dir, device="cpu", dtype=None):
        return cls(), None

    def predict_record(self, internal, use_pride=False):
        LAST_KWARGS["use_pride"] = use_pride
        n = len(internal["questions"][0]["options"])
        probs = ([0.7] + [0.3 / (n - 1)] * (n - 1)) if n > 1 else [1.0]

        class _P:
            def tolist(self):
                return list(probs)
        return [(_P(), _P())]

    def eval(self):
        return self
'''


def write_stub_ckpt(tmp_path: Path) -> Path:
    ckpt = tmp_path / "ckpt"
    ckpt.mkdir(exist_ok=True)
    (ckpt / "model.py").write_text(STUB_MODEL_PY, encoding="utf-8")
    return ckpt


def fake_torch(monkeypatch) -> SimpleNamespace:
    fake = SimpleNamespace(set_num_threads=lambda n: None,
                           float32="fp32", load=lambda *a, **k: {})
    monkeypatch.setitem(sys.modules, "torch", fake)
    return fake


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


def make_adapter(tmp_path, base=None, pride=False, date_facts=True):
    ad = rj.MiniCPMAdapter(write_stub_ckpt(tmp_path), base, "fp32", 4,
                           600.0, pride, date_facts)
    ad.load()
    return ad


def test_load_default_path_uses_their_load_checkpoint(tmp_path, monkeypatch):
    fake_torch(monkeypatch)
    ad = make_adapter(tmp_path)
    assert ad.model is not None and ad.tok is None
    assert ad.warnings == []


def test_load_local_base_overrides_head_pt_base(tmp_path, monkeypatch):
    fake = fake_torch(monkeypatch)
    fake.load = lambda *a, **k: {"lora_r": 16, "head_dim": 256,
                                 "lora_targets": "all", "top_k_layers": 0,
                                 "temperature": 1.1314}
    ad = rj.MiniCPMAdapter(write_stub_ckpt(tmp_path), tmp_path / "base",
                           "fp32", 4, 600.0, False, True)
    ad.load()
    kwargs = dict(ad._model_mod.LAST_KWARGS)
    assert kwargs["base_model"] == str(tmp_path / "base")
    assert kwargs["lora_r"] == 16
    assert kwargs["adapter_dir"] == str(tmp_path / "ckpt")
    assert ad.model.head.temperature == 1.1314


def test_load_head_state_dict_failure_is_a_warning(tmp_path, monkeypatch):
    fake = fake_torch(monkeypatch)
    fake.load = lambda *a, **k: {"lora_r": 16, "temperature": 1.0,
                                 "state_dict": {"bad": "shape"}}
    ad = rj.MiniCPMAdapter(write_stub_ckpt(tmp_path), tmp_path / "base",
                           "fp32", 4, 600.0, False, True)
    ad.load()
    assert ad.warnings == ["head state_dict: size mismatch"]
    # the run continues with the fitted temperature regardless
    assert ad.model.head.temperature == 1.0


def test_run_noul_maps_false_slot_to_no_label(tmp_path, monkeypatch):
    stub_decision_result(monkeypatch, tmp_path)
    fake_torch(monkeypatch)
    ad = make_adapter(tmp_path)
    res = ad.run(make_task("noul", {"true": "held", "false": "not held"},
                           ["no", "yes"], "Was it upheld?"))
    assert res.ok and res.error is None
    assert res.probs_source == "native"
    assert res.label == "no"  # 0.7 mass on slot 0 = "no"
    assert list(res.probs) == ["no", "yes"]
    assert abs(sum(res.probs.values()) - 1.0) < 1e-9
    # wire reached their converter with the serving-default date facts
    assert ad._model_mod.LAST_WIRE["add_date_facts"] is True
    assert ad._model_mod.LAST_KWARGS["use_pride"] is False
    assert res.latency_s is not None


def test_run_choice_three_way_keys_probs_by_labels(tmp_path, monkeypatch):
    stub_decision_result(monkeypatch, tmp_path)
    fake_torch(monkeypatch)
    ad = make_adapter(tmp_path)
    res = ad.run(make_task("choice", {"v-a": "ships in 2 days",
                                      "v-b": "ships in 5 days",
                                      "v-c": "never ships"},
                           ["v-a", "v-b", "v-c"],
                           "Which vendor ships first?"))
    assert res.ok and res.label == "v-a"
    assert list(res.probs) == ["v-a", "v-b", "v-c"]


def test_run_flags_reach_their_code(tmp_path, monkeypatch):
    stub_decision_result(monkeypatch, tmp_path)
    fake_torch(monkeypatch)
    ad = make_adapter(tmp_path, pride=True, date_facts=False)
    res = ad.run(make_task("score", ["breach", "partial"],
                           ["0", "1"], "Rate it."))
    assert res.ok
    assert ad._model_mod.LAST_WIRE["add_date_facts"] is False
    assert ad._model_mod.LAST_KWARGS["use_pride"] is True


def test_run_rejects_unrenderable_question(tmp_path, monkeypatch):
    stub_decision_result(monkeypatch, tmp_path)
    fake_torch(monkeypatch)
    ad = make_adapter(tmp_path)
    res = ad.run(make_task("essay", None, ["a", "b"], "Write."))
    assert not res.ok and res.error == "unrenderable_question"


def test_run_rejects_slot_count_mismatch(tmp_path, monkeypatch):
    stub_decision_result(monkeypatch, tmp_path)
    fake_torch(monkeypatch)
    ad = make_adapter(tmp_path)
    ad._model_mod = SimpleNamespace(
        to_internal_record=lambda rec, add_date_facts=True: {
            "questions": [{"options": ["x", "y"]}]})
    ad.model = SimpleNamespace(
        predict_record=lambda internal, use_pride=False: [
            (None, SimpleNamespace(tolist=lambda: [0.6, 0.4, 0.0]))])
    res = ad.run(make_task("noul", None, ["no", "yes"], "Was it upheld?"))
    assert not res.ok and res.error == "slot_count_mismatch"


def test_run_model_exception_classified_as_error(tmp_path, monkeypatch):
    stub_decision_result(monkeypatch, tmp_path)
    fake_torch(monkeypatch)
    ad = make_adapter(tmp_path)

    def boom(*a, **k):
        raise ValueError("boom")
    ad._model_mod = SimpleNamespace(to_internal_record=boom)
    res = ad.run(make_task("choice", {"a": "x", "b": "y"}, ["a", "b"],
                           "Pick."))
    assert not res.ok and res.error == "ValueError"
