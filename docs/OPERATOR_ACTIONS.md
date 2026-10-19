# Operator Action Checklist — 2026-10-19

## Immediately actionable

### 1. PR #7 — merge cube games branch
- **Branch:** `site/games-audit-2026-10` (gitforge + origin)
- **What it adds:** `clock-core.js` + `clock.html` (TNoodle Jaumann/Wethiell table), `cube-n-core.js` (2×2 BFS coach, 4–21×N free play), `pyraminx-core.js` + `skewb-core.js` (Tier 1 shipped)
- **Action:** Review + merge at https://github.com/aliasfoxkde/OpenCodifier/pull/7

### 2. Site deploy — merge vision branch
- **Branch:** `vision/vision-rung-2026-10` (gitforge)
- **What it adds:** WCAG 2.1 AAA audit (SITE_WCAG_AUDIT.md), §81 closed in SPEC_COVERAGE, Phase 21/22 closed in PLAN.md
- **Action:** Either merge to `main` OR push `site/*` branch to trigger CF Pages preview

### 3. X/Discord handles — add to site
- Pages currently render nothing for X/Discord links
- Add real handles to `site/index.html` / `site/site-core.js` footer

---

## T5500-dependent — do when node is powered on

### 4. VIVERE E1 arms training (#88, #169)
- **Corpus:** `~/oc-model-eval/corpora/e1-sft-v2/` (177,999 rows, sha verified)
- **Bodies staged:** 2B + 4B Qwen3.5 + LoRA
- **Trainer:** `runner/e1_train.py` on fedora → night chain → gate eval → JevBench conditional leg
- **Gates:** suite ≥ 0.68 AND JevBench ≥ 0.6494 before any HF publish
- **T5500 precondition:** power on 2×V100, set `nvidiaia_powershift.sh` (120W caps per GPU)

### 5. jabr upstream PR
- **Adapter:** `bench/backends/opencodifier.py` in `/nas/Temp/repos/jabr-classifier-benchmark/`
- **Already on board:** Board C (v1 0.346 micro, v2 0.400 micro, 866/866, 0 errors, 1.6 ms/case)
- **Action:** Submit PR to `jabr/classifier-benchmark` upstream

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

## Git state
- `vision/vision-rung-2026-10` → gitforge @ `50fe817` (clean, pushed)
- `site/games-audit-2026-10` → gitforge + origin (PR #7 open)
