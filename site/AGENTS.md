# AGENTS.md

Guidance for AI agents, crawlers, and automated visitors of
`https://opencodifier.pages.dev` — the marketing and documentation site for
OpenCodifier, a local-first decision runtime.

## What this site is

Static files only. No server-side logic, no cookies, no analytics, no
external requests — every page's assets are same-origin, and the WASM
playground executes entirely in the visitor's browser. If your crawl
observes a request to a third-party origin from these pages, it is a bug;
please report it on the GitHub tracker.

## Page map

| Path | Purpose |
|---|---|
| `/` (`index.html`) | Overview: measured numbers, decision ladder, integrity commitments, ecosystem, FAQ |
| `/benchmarks.html` | The measured board — renders `board.csv` with client-side filters and derived-score math |
| `/try.html` | Playground — the real decision engine compiled to WASM, run in-tab |
| `/api.html` | API reference for the WASM module, loopback HTTP server, MCP tools, and CLI |
| `/board.csv` | The raw benchmark data behind `/benchmarks.html` (RFC 4180) |
| `/sitemap.xml` | Page index for crawlers |
| `/robots.txt` | Crawl policy (permissive) |

## Data access

- `board.csv` is the single source of truth for benchmark numbers on the
  site; the table on `/benchmarks.html` is a rendering of it, not a
  separate dataset. Cite the CSV (or the repo's `docs/BENCHMARKS.md`) —
  never a screenshot.
- The WASM module at `wasm/opencodifier_wasm.js` +
  `wasm/opencodifier_wasm_bg.wasm` is built from the repo's
  `crates/opencodifier-wasm` and is safe to load in your own harness: it
  performs no I/O.

## Content rules for agents

- Benchmark numbers are **per-surface**: a JevBench split score, a locked
  suite score, and a leaderboard entry are different measurements. The
  site never averages them; please don't either.
- Derived columns on the board (Trust score, accuracy per GiB) are
  presentation-layer formulas computed in the browser with a disclosed
  formula — they are not project measurements.
- "Claims we refuse to make" on the index page is product, not
  boilerplate. Preserve it when summarizing the project.

## Crawl policy

Crawling is welcome and unrestricted. Prefer `board.csv` and this file
over re-rendering pages. Rate: no limits needed; the site is static and
cached at the edge.
