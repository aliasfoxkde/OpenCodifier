# OpenCodifier marketing site

Static single-page site -- deploys make **zero external requests** (system fonts,
inline SVG, committed CSS, no tracking). Every number on the page traces to a
committed benchmark run (`docs/BENCHMARKS.md`,
`benchmarks/decision-model/results/REPORT.md`, `docs/RESEARCH.md S.14`).

## The board page (`benchmarks.html`)

Renders `board.csv` -- a synced copy of
`benchmarks/decision-model/results/board.csv` -- client-side (`benchmarks.js`,
quote-aware RFC 4180 parser, no dependencies). Filters (search, surface chips,
quantization, tier, provenance, has-ECE / has-size) and the derived metrics
(Trust score with an adjustable lambda, accuracy-per-GiB) are computed in the
browser; the formulas are disclosed on the page and ranking is **per benchmark
surface**, never across them (`docs/RESEARCH.md S.14.1`).

After regenerating the results CSV, sync and redeploy:

```bash
just site-board                     # cp results/board.csv -> site/board.csv
wrangler pages deploy site --project-name=opencodifier
```

The page must never hand-type a number: if a figure isn't in the CSV or the
docs of record, it doesn't ship.

## The playground (`try.html`) and API reference (`api.html`)

`try.html` runs the real engine (`crates/opencodifier-wasm`, zero-ML stack)
compiled to WebAssembly, entirely in the visitor's tab. `site/wasm/` is a
**build artifact, not source** -- it is gitignored and rebuilt before every
deploy:

```bash
just site-wasm                       # wasm-pack build --target web -> site/wasm
wrangler pages deploy site --project-name=opencodifier
```

The glue's async initializer is exported as the **default** export
(`__wbg_init as default`) -- `demo.js` calls it as `mod.default()`, not
`mod.init()`. `api.html` documents the implemented surface only (routes,
error codes, and envelope shapes are checked against the crate sources);
its "run it" buttons hand a fixture to the playground via the
`oc-preset` sessionStorage key.

## Brand assets (`assets/`)

`brand-mark.svg` is the canonical vector mark -- geometry extracted from
`docs/images/OpenCodifier_Logo.svg` (paths `apg-1`/`apg-2`, the export's
nested transforms baked into two plain ones, off-canvas design-tool clutter
dropped) with the brand gradient `#728ee4 -> #3a00ad`. The nav and the
favicon use it; `favicon.svg` adds the dark rounded tile for legibility at
16 px. `brand-mark.png`, `brand-label.png`, `brand-lockup.png` are
transparency-keyed extractions of the raster art in `docs/images/` (alpha =
max of the RGB channels, unpremultiplied color, so the neon-glow falloff
survives on both themes); `og-image.png` (1200x630, referenced by every
page's og/twitter meta) composes the lockup. If the `docs/images/` art
changes, re-extract -- never hand-edit the derived assets.

## Local preview

```bash
python3 -m http.server 8123 --directory site
# open http://127.0.0.1:8123
```

## CSS pipeline (Tailwind, dev-time only)

Styling is two committed stylesheets: `styles.css` (hand-written component
layer, tokens in `:root`) and `tw.css` (Tailwind v4 build -- theme tokens +
utilities, preflight skipped so the component layer renders identically).
The Tailwind source is `src/tw.css`; rebuild after editing it or after
adding new utility classes to the HTML:

```bash
cd site && npm install        # once
npm run build:css             # writes tw.css -- commit the artifact
```

The build runs at dev time only; `node_modules/` is gitignored and deploys
never load a CDN. `package-lock.json` is committed.

## Deploy (Cloudflare Pages)

```bash
wrangler pages project create opencodifier --production-branch main   # once
wrangler pages deploy site --project-name=opencodifier
```

Direct-upload project; `site/` is the deploy directory. Auth comes from
`CLOUDFLARE_API_TOKEN` in the environment (already configured on this host).

## Editing rules

- **No unnumbered claims.** If a sentence states a figure, it exists in the
  docs of record; link or footnote it. The "Claims we refuse to make" card is
  part of the product -- keep it.
- **No external requests.** System fonts, inline SVG, no CDNs -- the page must
  work offline, like the product it sells.
- **No guessed links.** Every outbound URL is verified against the target
  project's own repo/deploy before it ships. Links pending a verified URL
  stay out of the page (the machinery card names them without links).
- One number per benchmark surface, always labeled (see `docs/RESEARCH.md S.14.1`
  for the cross-surface divergence table).
