# GitForge nested-BEGIN fix + OC green pipeline — 2026-11-02 (overnight session)

Outcome: both goals landed. GitForge fix deployed; OpenCodifier pipeline run
`8a386eea-b965-4859-bae1-4a3774b6dd33` on tree `b9a6ddf4` (identical to
`c844a4b`) **succeeded** — all six lanes green (lint, test, doc, e2e,
supply-chain, coverage). Run-level status read from
`/var/lib/gitforge/gitforge.db` `pipeline_runs` at 2026-11-02 09:15:04 UTC
(3:15 AM CST).

## GitForge nested-BEGIN defect (fixed)

- Defect: transaction nesting bug (`BEGIN` inside an open `BEGIN IMMEDIATE`)
  in `gitforge-db` connection handling caused CI triggers after a scheduler
  restart to lose their job rows — a pushed commit planned a pipeline whose
  jobs never persisted, so the run sat with nothing to execute.
- Fix: built at `be51184d` (GitForge repo), deployed as release
  `gitforge-be51184d-20261102`; all four services (api, git-server, runner,
  ci) verified on the new binaries via live API on port 42780.
- Verification cost: three OC retrigger pushes (`990ebdc`, `4706b7c`,
  `5607cb6` — all empty, same tree) before one ran cleanly. The first
  post-deploy push still lost its job rows (deploy-window recovery raced the
  push); the third persisted all six jobs and the run went green. The fix is
  proven by that final run; earlier failures were deploy-window artifacts.
- Residual (recovers on its own): during the durable-trigger sweep of ~100
  interrupted-run workspaces, job-row writes starve behind `BEGIN IMMEDIATE`
  contention for a few seconds at a time. Post-sweep, writes are normal.

## Zombie-run backlog finding (no action needed)

Post-deploy, the restarted scheduler drained a pre-deploy trigger storm:
112 `running` pipeline runs (53 on Burst-Compute-Testing — the same 3 commits
retriggered in a loop — and 4 on Drone-Flight-Data-Processing). Operator
approved "cancel zombies only"; reachability checks (`git merge-base
--is-ancestor <commit> main`) showed **every stuck commit is reachable from
its repo's main**, so none met the cancellation bar. The scheduler is
draining the backlog serially on its own (backdrop: 112→57 running within
minutes of observation). No intervention taken; it self-resolves.

## Local aegis scan (closed)

Local `aegis scan` rc=1 findings were exclusively in gitignored benchmark
result JSONs and `.claude/scheduled_tasks.lock` — untracked content invisible
to CI's in-image scan. No repo change warranted.

## Empty retrigger commits

`990ebdc`, `4706b7c`, `5607cb6` on OC main are empty (same tree `b9a6ddf4`);
history is clean, only commit-count noise. No rewrite (pushed history stays
immutable per standing rule).
