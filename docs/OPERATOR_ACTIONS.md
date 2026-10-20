# Operator Action Checklist — 2026-10-19

## Immediately actionable

### 1. PR #7 — merge cube games branch
- **Branch:** `site/games-audit-2026-10` (gitforge + origin)
- **What it adds:** `clock-core.js` + `clock.html` (TNoodle Jaumann/Wethiell table), `cube-n-core.js` (2×2 BFS coach, 4–21×N free play), `pyraminx-core.js` + `skewb-core.js` (Tier 1 shipped)
- **Status:** ✅ MERGED and deployed to CF Pages production (2026-10-19)

### 2. Vision branch — merge to main
- **Branch:** `vision/vision-rung-2026-10` (gitforge, current branch)
- **What it adds:** WCAG 2.1 AAA audit (SITE_WCAG_AUDIT.md), §81 closed in SPEC_COVERAGE, Phase 21/22 closed in PLAN.md, SigLIP2 ONNX vision probe passing
- **Action:** Merge to `main` and push — gitforge CI will deploy

### 3. X/Discord handles — add to site
- Pages currently render nothing for X/Discord links
- Add real handles to `site/index.html` / `site/site-core.js` footer

---

## T5500-dependent — done

### 4. VIVERE E1 arms training (#88) — ✅ BOTH GATES PASSED
- **T5500 powered on 2026-10-19**
- **E1-E 2B:** suite holdout 0.8083 ≥ 0.75 GATE_PASS; JevBench 231 0.6840 ≥ 0.6494 GATE_PASS
- **Artifacts:** `~/t5500-bundle/runs/e1-e-qwen2b/adapter-best/`
- **JevBench manifest:** `~/t5500-bundle/eval-results/jevbench-slot-e1e-best-231/`
- **Status:** First own arm to pass both release-gate legs. Ready for HF publish when operator approves.

### 4b. VIVERE MiMo extraction (GPU 1)
- GPU 1 available for teacher scoring with MiMo on T5500
- T5500 GPU0 available for VIVERE work

### 5. jabr upstream PR — ✅ DONE
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
- `vision/vision-rung-2026-10` → gitforge @ `0fdca52` (clean, pushed)
  - **2026-10-20 additions:** NCM audit (§23 in RESEARCH.md), proof-first fusion measurement (0/231 proofs, fusion ceiling = model alone), docs/references/NCM-next-concept-modeling.md
- `site/games-audit-2026-10` → gitforge + origin (PR #7 MERGED)
