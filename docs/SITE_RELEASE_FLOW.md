# Site Release Flow (staging -> review -> production)

The marketing site (`site/`, published at opencodifier.pages.dev) ships on a
review-gated loop. Nothing lands on production without a human looking at the
exact bytes that will deploy. This document is the process of record.

## The loop

```
play on the site
  -> rate a game (stars + feedback, localStorage)
  -> "file an issue" prefills a GitHub issue from that rating
  -> scheduled improvement pass turns labeled issues into commits
       on a feature branch
  -> branch push = staging (CI + Cloudflare Pages preview URL)
  -> human reviews the preview in a browser
  -> approved: merge the PR -> main -> production deploy
```

## Branch model

| Ref | Role | Where |
|---|---|---|
| `main` | production. CF Pages deploys it automatically. | origin (GitHub) |
| `site/<topic>` | staging. Gets a CF Pages branch preview per push. | GitForge + origin |
| never | direct pushes to `origin main` (ruleset-protected). | -- |

Rules:

- Feature branches are named `site/<topic>` (`site/gridwars`,
  `site/improve-pass-2026-11`). One topic per branch; the scheduled
  improvement pass gets one branch per run.
- Every branch push goes to **GitForge first** (CI runs there), then origin
  (preview + PR). GitForge is the primary CI/CD platform; GitHub is the
  public mirror and PR surface.
- No force pushes, no history rewrites of pushed commits. Fix forward.

## Staging: how a branch becomes reviewable

1. **Local check** (fast, run before every push):

   ```
   cd site
   for f in tests/*.mjs; do node --test "$f" || exit 1; done
   python3 -m http.server 8777   # review server at http://localhost:8777
   ```

2. **Push the branch**:

   ```
   git push gitforge <branch>
   git push origin <branch>
   ```

3. **CI** runs the GitForge pipeline (`.gitforge.yml`). The current image
   (`opencodifier-ci-rust:2`) carries no Node, so the site's JS test suite is
   not yet a CI lane -- it runs locally and in the improvement pass instead.
   Adding the lane requires a Node-bearing CI image first; do not fake it with
   a job that cannot run.

4. **Cloudflare Pages** builds every pushed branch automatically. The preview
   URL (`<branch>.opencodifier.pages.dev`) IS staging. It serves the exact
   commit; there is no separate staging build step.

## Review checklist (human, in a browser)

Work down this list on the preview URL before approving. A checkmark means
"verified, not assumed".

- [ ] Every page loads with a clean console (no errors, no 404s).
- [ ] Games boot and play: seed a new run, let the bot/ball/decision loop run.
- [ ] Layout at three widths: 375, 900, 1400 px. No overlap, no clipped panels.
- [ ] Accordion rule: opening one `<details>` closes the rest (site-wide).
- [ ] Site chrome intact: nav, footer, game switcher, TOC rail.
- [ ] Zero network calls at runtime beyond the page load itself (static site;
      leaderboards are session-only, ratings live in localStorage).
- [ ] No secrets, tokens, or credentials anywhere in the diff.
- [ ] `site/tests/*.mjs` all green locally.

## Promotion: staging -> production

Approval is the PR merge. Merging is a human action; the agent never merges
its own work.

1. Open the PR on GitHub from the `site/<topic>` branch.
2. The reviewer uses the preview URL + the checklist above.
3. On approval, merge the PR. `main` moves; CF Pages deploys production.
4. Sync the merge back to GitForge (push `main` there) so both remotes agree.
5. Spot-check the production URL after the deploy finishes (hard refresh).

## Scheduled improvement pass

A periodic (suggested: monthly) agent run that turns accumulated feedback into
one reviewable branch. Inputs:

- GitHub issues labeled `game-feedback` (created via each game page's
  "file an issue" button, which prefills stars + verbatim feedback).
- The `Claims we refuse to make` card and benchmark pages of record
  (`docs/BENCHMARKS.md`) -- copy must never drift past what is measured.

Outputs, per run:

- One branch `site/improve-pass-<YYYY-MM>`.
- One commit per issue, message `fix(site): <what> (fixes #N)` so the issue
  closes on merge.
- The standard checklist run locally, then the branch pushed for review.
- Findings logged in `docs/planning/playground/FINDINGS.md`.

The pass never: bumps claimed numbers without a recorded benchmark, adds a
backend call, widens scope past the labeled issues, or merges its own PR.

## Related

- `docs/BENCHMARKS.md` -- comparison page of record; one number per surface.
- `.gitforge.yml` -- CI pipeline (Rust lanes today; Node lane pending image).
- `site/tests/` -- Node test suite per game/feature module.
