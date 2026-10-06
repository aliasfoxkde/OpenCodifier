# OpenCodifier marketing site

Static single-page site — deploys make **zero external requests** (system fonts,
inline SVG, committed CSS, no tracking). Every number on the page traces to a
committed benchmark run (`docs/BENCHMARKS.md`,
`benchmarks/decision-model/results/REPORT.md`, `docs/RESEARCH.md §14`).

## Local preview

```bash
python3 -m http.server 8123 --directory site
# open http://127.0.0.1:8123
```

## CSS pipeline (Tailwind, dev-time only)

Styling is two committed stylesheets: `styles.css` (hand-written component
layer, tokens in `:root`) and `tw.css` (Tailwind v4 build — theme tokens +
utilities, preflight skipped so the component layer renders identically).
The Tailwind source is `src/tw.css`; rebuild after editing it or after
adding new utility classes to the HTML:

```bash
cd site && npm install        # once
npm run build:css             # writes tw.css — commit the artifact
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
  part of the product — keep it.
- **No external requests.** System fonts, inline SVG, no CDNs — the page must
  work offline, like the product it sells.
- **No guessed links.** Every outbound URL is verified against the target
  project's own repo/deploy before it ships. Links pending a verified URL
  stay out of the page (the machinery card names them without links).
- One number per benchmark surface, always labeled (see `docs/RESEARCH.md §14.1`
  for the cross-surface divergence table).
