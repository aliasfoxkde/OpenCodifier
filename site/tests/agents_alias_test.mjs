/* _redirects must alias every case permutation of AGENTS.md to the canonical
   file — a wrong-cased fetch (agents.md, AGENTS.MD, Agents.md, …) resolves
   instead of 404ing. Structural test: parses the generated rules file. */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");

const CANON = "/AGENTS.md";

function rules() {
  const out = new Map();
  for (const line of readFileSync(join(root, "_redirects"), "utf8").split("\n")) {
    const t = line.trim();
    if (!t || t.startsWith("#")) continue;
    const [src, dst, code] = t.split(/\s+/);
    out.set(src, { dst, code });
  }
  return out;
}

test("canonical AGENTS.md exists on disk", () => {
  assert.ok(readFileSync(join(root, "AGENTS.md"), "utf8").length > 0);
});

test("every alias points at the canonical with a permanent redirect", () => {
  for (const [src, { dst, code }] of rules()) {
    assert.equal(dst, CANON, src);
    assert.equal(code, "301", src);
  }
});

test("all 2^6 stem case permutations covered for .md and .MD", () => {
  const have = rules();
  const stems = new Set();
  const rec = (prefix, rest) => {
    if (!rest.length) { stems.add(prefix); return; }
    rec(prefix + rest[0].toLowerCase(), rest.slice(1));
    rec(prefix + rest[0].toUpperCase(), rest.slice(1));
  };
  rec("", "agents");
  for (const s of stems) {
    if (`/${s}.md` !== CANON) assert.ok(have.has(`/${s}.md`), `missing /${s}.md`);
    assert.ok(have.has(`/${s}.MD`), `missing /${s}.MD`);
  }
  assert.equal(stems.size, 64);
});

test("canonical is not aliased to itself; no duplicates", () => {
  const lines = readFileSync(join(root, "_redirects"), "utf8").split("\n")
    .map((l) => l.trim()).filter((l) => l && !l.startsWith("#"));
  const sources = lines.map((l) => l.split(/\s+/)[0]);
  assert.ok(!sources.includes(CANON));
  assert.equal(new Set(sources).size, sources.length, "duplicate source rules");
});
