# Operator Action Checklist — 2026-10-21

## Immediately actionable

### 1. PR #7 — merge cube games branch ✅
- **Branch:** `site/games-audit-2026-10` (gitforge + origin)
- **Status:** ✅ MERGED and deployed to CF Pages production (2026-10-19)

### 2. Vision branch — merge to main ✅
- **Branch:** `vision/vision-rung-2026-10`
- **Status:** ✅ MERGED to `main` and pushed to gitforge (2026-10-20)

### 3. X/Discord handles — add to site
- Pages currently render nothing for X/Discord links
- Add real handles to `site/index.html` / `site/site-core.js` footer

### 4. Sandbox EROFS fix (local CI — operator action)
- `cargo clippy` and `cargo test` fail with rc=101 when routed through the
  harness job manager's sandbox (`ProtectSystem=strict ReadWritePaths`)
  because `$PWD/target/` is not in the allowlist.
- **Fix:** add `$PWD/target/` to the `ProtectSystem=strict ReadWritePaths=`
  directive in the systemd unit that starts the job manager service.
- Without this fix, the local `just ci` gate cannot pass; GitForge CI
  (which uses its own sandbox) is unaffected.

---

## T5500-dependent

### 5. E1-F / E1-G arms training (GPU 1 + GPU 0) — IN PROGRESS
- **Started:** 2026-10-20 16:37 UTC (re-launched after SSH session bug)
- **E1-F (GPU 1):** corpus e1-softlabel-v2 (187,822 rows, teacher distributions from MiMo).
  Teacher-gold argmax agreement 96.1–97.7%. Attacks accuracy AND calibration.
- **E1-G (GPU 0):** corpus merged-v6-full (306,999 rows, Jev-distilled stream).
  Includes 129,000 rows whose labels were distilled from Jev 1.13 itself.
- **CORPUS NOTE (v3 corpus analysis, 2026-10-21):** `SargeDev/jev-distill-corpus-v3`
  (740,957 rows) has **zero family overlap** with JevBench-231's taxonomy
  (`relational_compositional`, `lexical_semantic`, `metadata_match`). E1-G's
  merged-v6 corpus is the typed-decisions stream (same decision schema as JevBench)
  — the more relevant training signal. v3's yuri_v3 families (technical, agent,
  knowledge, business, medical, genomics, physics…) do not map to eval families.
  The path forward is (a) upweight the typed-decisions stream, (b) remap v3 families
  to eval taxonomy, or (c) per-family routing via `family: Option<String>` in the
  request IR. See RESEARCH.md §29.
- **Knobs:** Qwen3.5-2B + verdict-slot LoRA (r=16, alpha=32, dropout=0.05,
  lr=1e-4, batch=2, accum=8, max_len=2048, fp16, sdpa, seed=42, 1 epoch).
- **Chain:** train → suite holdout eval → JevBench public 231 slot readout.
- **Gates:** suite ≥ 0.75 AND JevBench 231 ≥ 0.6494 (E1-E champion is 0.8083 / 0.6840).
- **LEAKAGE AUDIT REQUIRED before reporting any v6 number:** merged-v6-full
  contains Jev-distilled labels; if any JevBench-231 item leaked, the number
  is contaminated and must be disclosed.
- **Logs:** `~/t5500-bundle/logs/e1fg-{e1f-soft,e1g-v6}.out` (outer launcher),
  `e1fg-{e1f-soft,e1g-v6}-train.log` (trainer).
- **Script:** `~/t5500-bundle/train_e1fg.sh` (Usage: `./train_e1fg.sh <e1f-soft|e1g-v6> [gpu_index]`).

### 6. HF release of E1-E 2B — awaiting operator approval
- **Gates:** suite holdout 0.8083 ≥ 0.75 ✅; JevBench 231 0.6840 ≥ 0.6494 ✅
- **Artifacts:** `~/t5500-bundle/runs/e1-e-qwen2b/adapter-best/`
- **Release requires:** operator approval + org name; Apache-2.0; corpus withheld; explicit opt-in pull only, never auto-download at build.

### 5 (prev). jabr upstream PR — ✅ DONE
- **PR:** https://github.com/aliasfoxkde/classifier-benchmark/pull/1 (fork `aliasfoxkde/classifier-benchmark`, branch `opencodifier-backend`)
- **Upstream:** needs review + merge by jabr maintainers at `jabr/classifier-benchmark`

---

## Verification commands (run on this host)

```bash
# CI gates
cd /nas/Temp/repos/OpenCodifier
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
aegis scan --file crates/ --quiet

# Vision probe (requires ORT_DYLIB_PATH)
ORT_DYLIB_PATH=~/.local/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.so.1.24.2 \
  ./target/release/opencodifier decide \
    -i /tmp/vision-req.json \
    --vision-model-dir /nas/Temp/work/oc-model-eval/models/siglip2-base-patch16-256-ONNX \
    --image /tmp/blue.png
```

---

## JevBench split results (2026-10-19)

| split | n_attempted | n_valid | accuracy | ECE | p50 latency | determinism |
|---|---|---|---|---|---|---|
| easy | 48 | 48 | 0.6458 | 0.270 | 3.8 ms | 48/48 ✅ |
| full (public subset) | 72 | 46 | 0.2917 | 0.240 | 5.0 ms | 72/72 ✅ |
| hard | 53 | 47 | 0.2264 | 0.682 | 10.9 ms | — |

- Easy split: strong on adequacy (0.50) and policy (0.42); weak on extraction (0.33) and intent (0.25)
- Full (subset): ordinal family 0/12 schema-valid; extraction ECE 0.63 (confident + wrong)
- Hard split: 47.7% coverage (58 items not attempted); multi_hop 0.00, temporal_numeric 0.10
- Determinism: engine is bit-reproducible — all splits pass the replay check
- Raw runs at `/nas/Temp/work/oc-model-eval/runs/jevbench/engine-{easy,full,hard}/`

---

## Git state
- `main` → gitforge @ `d75df6d` (clean, pushed)
  - **2026-10-22:** Fixed stale HTTP bench note (§27), fixed calibration artifact path in upload script, updated OPERATOR_ACTIONS
- `vision/vision-rung-2026-10` → gitforge @ `d75df6d` (synced with main)
- `site/games-audit-2026-10` → gitforge + origin (PR #7 MERGED)

---

## Results this session (2026-10-22)

| Item | Result |
|---|---|
| E1-F/E1-G arms | **Running.** E1-F ~28% (step 3260/11615), loss 0.2659; E1-G ~23% (step 4480/19058), loss 0.1292. No NaNs. |
| §27 HTTP bench | **Already existed.** Fixed stale note in RESEARCH.md — `crates/opencodifier-http/benches/http.rs` has bench_http_overhead. |
| Upload script fix | **Fixed.** Calibration artifact path corrected to `e1e-qwen3.5-2b-holdout.json`. |
| JEV-27B corpus discovery | **New lane.** `SargeDev/jev-distill-corpus-v3` (740,957 rows) with `family`/`domain`/`target` fields. yuri_v3 (498k rows) from Jev 1.13; openjev_v2 (94k, CC0). |
| Training pipeline improvements | **Identified.** Multi-teacher aggregation, domain-specific adapters, quality-weighted curriculum, fast iteration on 1000-item stratified slice. |

---

## JEV-27B corpus — action items

Priority for next session:

1. **Probe JEV-27B on JevBench-231** — `autotrust/JEV-27B` (Qwen3.8-27B + 109M LoRA) as contamination audit baseline
2. **Train on v3 corpus directly** — 740k rows with Jev 1.13 soft labels; replace/augment E1-G's merged-v6
3. **Use openjev_v2 for clean eval** — 94k CC0 rows with programmatic ground truth
4. **VIVERE extract from JEV-27B on contested items** — E1-E slot_agree ~0.98; mine the ~2% disagreement for training signal
