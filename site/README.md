# OpenCodifier marketing site

Static single-page site — no build step, no external requests, no tracking.
Every number on the page traces to a committed benchmark run
(`docs/BENCHMARKS.md`, `benchmarks/decision-model/results/REPORT.md`,
`docs/RESEARCH.md §14`).

## Local preview

```bash
python3 -m http.server 8123 --directory site
# open http://127.0.0.1:8123
```

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
  part of the product — keep it.
- **No external requests.** System fonts, inline SVG, no CDNs — the page must
  work offline, like the product it sells.
- One number per benchmark surface, always labeled (see `docs/RESEARCH.md §14.1`
  for the cross-surface divergence table).
