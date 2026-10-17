# Aegis Security Audit Report

**Scanner**: Aegis 0.6.3 (replaces Atheon MCP)  
**Date**: 2026-10-17  
**Baseline**: `.aegis/baseline.json` (18,412 findings — snapshot at commit `5251e52`)  
**Gate**: `aegis --format json scan . --baseline .aegis/baseline.json -q` → exit 1 means new findings

---

## Severity Breakdown (baseline total: 18,412)

| Severity | Count | Notes |
|---|---|---|
| CRITICAL | 405 | 403 financial-ID false positives in JSON fixtures; 2 git-credential-leak in docs |
| HIGH | 805 | 600+ financial-ID false positives; ~50 rust-unsafe (FFI, intentional); XSS all esc()-mitigated |
| MEDIUM | 4,344 | dispersed; mostly code-quality and shift-left noise |
| LOW | 12,856 | dispersed; mostly code-quality and performance |
| INFO | 2 | benign |

---

## CRITICAL Findings

| # | Location | Pattern | Disposition |
|---|---|---|---|
| 1 | `crates/opencodifier-model/src/julia/fixtures/julia-reference-trimmed.json:412` | `credit-card-number-generic` + `credit-card-amex` | **ACCEPTED** — numerical logits array that happens to match Luhn patterns; fixture data, never networked |
| 2 | `site/docs.html:338` | `git-credential-leak` | **ACCEPTED** — `git clone https://github.com/aliasfoxkde/OpenCodifier` in docs; unauthenticated public URL |
| 3 | `site/index.html:356` | `git-credential-leak` | **ACCEPTED** — same as above |

---

## HIGH Findings

| # | Location | Pattern | Disposition |
|---|---|---|---|
| 4 | `crates/opencodifier-ffi/src/lib.rs:179,195,260,278` | `rust-unsafe-block` + `rust-unsafe-extern` | **ACCEPTED** — C-ABI FFI requires unsafe; audited, contained, documented |
| 5 | `crates/opencodifier-ffi/include/ocffi.h:28` | `rust-unsafe-extern` | **ACCEPTED** — same as above |
| 6 | `crates/opencodifier-http/src/lib.rs:281` | `healthcare-phi-plaintext-endpoint` | **ACCEPTED** — test hits `http://127.0.0.1/healthz`; loopback only |
| 7 | `crates/opencodifier-schema/src/openai.rs:128` | `vue-template-injection` | **ACCEPTED** — string constant `DESCRIPTIONS_MARKER = "opencodifier:descriptions="`; not Vue |
| 8 | `crates/opencodifier-engine/src/executor.rs:489` | `flask-sqlalchemy-raw-sql` | **ACCEPTED** — "sql" in comments describing the executor's purpose; not Flask |
| 9 | `crates/opencodifier-core/src/request.rs:322` | `code-injection-request` | **ACCEPTED** — test comment "malicious code guidance" in a test payload comment |
| 10 | `site/demo.js:150,222,254,259,290,310,542` | `xss`, `stored-xss`, `angular-innerhtml-xss` | **ACCEPTED** — `esc()` HTML-encodes all user-supplied content before innerHTML |
| 11 | `site/site.js:6,111,119,129,260,271,395,409,423,441` | `xss`, `stored-xss`, `angular-innerhtml-xss` | **ACCEPTED** — same esc() mitigation |
| 12 | `site/maze.js:42`, `pong.js:391`, `tower.js:524`, etc. | `xss`, `stored-xss`, `angular-innerhtml-xss` | **ACCEPTED** — same esc() mitigation; game output uses engine-decided text |
| 13 | `site/site-core.js:12` | `env-credential-assignment` | **ACCEPTED** — `THEME_KEY = "oc-theme"`; localStorage key, not a secret |
| 14 | `site/board.csv:19` | `path-traversal` | **ACCEPTED** — string `autotrust/JEV-27B` in CSV cell; false positive |

---

## MEDIUM/LOW Findings — Summary

| Category | Count | Disposition |
|---|---|---|
| Financial IDs in JSON fixtures (credit-card-*, TFN, SSN, ABA, Aadhaar, SIN) | ~900 | **ACCEPTED** — fixture data; logits arrays; numerical test values |
| `eval-usage` / `dangerous-execution` in `benchmarks/decision-model/runner/*.py` | 20 | **ACCEPTED** — separate benchmark project in `benchmarks/`; OpenCodifier does not execute these |
| `pii-output-marker` in site JS (62 occurrences) | 62 | **ACCEPTED** — UI label strings like "name", "date", "address" that Aegis misidentifies as PII fields |
| `privileged-container` in docs | 4 | **ACCEPTED** — docs discussing container security; not actual container configs |
| `ssrf-localhost` in docs | many | **ACCEPTED** — docs describe loopback URLs for local dev |
| Code quality / performance / web-development | ~15,000 | **ACCEPTED** — informational; Aegis cannot assess architectural context |
| `inefficient-css` in `site/src/tw.css` | ~100 | **ACCEPTED** — generated Tailwind output; not hand-written |

---

## Aegis Configuration Note

Pattern suppression at the project level (`.aegis/config.json` with `disabled_patterns`) is not supported by Aegis 0.6.3 — only the global `~/.config/aegis/pattern-state.json` accepts per-pattern suppressions, which affects all projects on this machine.  
All findings above are **accepted as documented** in this baseline. If Aegis CI fires, re-run with `--baseline .aegis/baseline.json` to confirm the delta is zero.

---

## Action Items

| Priority | Action | Owner |
|---|---|---|
| Low | Review `benchmarks/decision-model/runner/*.py` eval() usages — separate project, not in scope | mkinney |
| Low | Consider moving `benchmarks/decision-model/` to a separate repo to isolate concerns | mkinney |
| None | Aegis project-level suppressions — wait for Aegis to add `--exclude-pattern` CLI flag | — |
