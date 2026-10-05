#!/usr/bin/env python3
"""Generate the deterministic candidate-conditioned decision benchmark suite.

The suite is committed byte-locked (suite.json). Re-running this generator
must reproduce those bytes exactly; verify with:

    python3 suite/generate_suite.py /tmp/suite.json && cmp suite/suite.json /tmp/suite.json

`--holdout` writes the out-of-sample companion (suite_holdout.json,
byte-locked the same way): a different seed for the generative classes
and the disjoint half of the class-B authoring table.

Every item is a Choice question in the OpenCodifier IR shape: a context, a
question, 4-6 candidates with id + description, and exactly one correct
candidate id. Ground truth holds by construction, not by annotation.

Three difficulty classes, chosen to expose where each mechanism in the
escalation ladder stops being sufficient:

- metadata_match        explicit attribute constraints over an in-context
                        catalog; a satisfier must be checked, not guessed
- lexical_semantic      routing by paraphrase; the correct candidate shares
                        little lexical surface with the context
- relational_compositional  multi-hop structure (dependency chains, counts,
                        ordering); the answer is computed from the structure,
                        and candidate descriptions carry no lexical hints

All randomness flows through one seeded Random; iteration is over sorted
keys; output is json.dump(sort_keys=True, indent=1) + trailing newline.
"""

from __future__ import annotations

import json
import random
import sys
from pathlib import Path

SEED = 20260926
SUITE_VERSION = 1

# The held-out companion suite (suite_holdout.json): same generator, same
# class shapes and balance — different draws. The gate refit (F28) was
# fitted on suite v1, so v1 numbers are in-sample and this suite carries
# the out-of-sample claim. Class B is hand-authored (a seed cannot make
# new utterances), so the table carries a second, disjoint pool of 40
# phrasings below; --holdout selects it and re-seeds the generative
# classes. Default invocation must reproduce suite.json byte-for-byte.
HOLDOUT_SEED = 20261005
HOLDOUT_SUITE_VERSION = 3
B_POOL_MAIN = slice(0, 40)
B_POOL_HOLDOUT = slice(40, 80)

INSTRUCTIONS = (
    "You are choosing from a fixed list of options. Read the context and "
    "answer with exactly one option id from the allowed choices. Do not "
    "invent options. If unsure, still pick the closest option."
)


# ---------------------------------------------------------------------------
# Class A — metadata_match
# ---------------------------------------------------------------------------

_A_DOMAINS = [
    {
        "name": "deployment target",
        "thing": "target",
        "question": "Which deployment target satisfies all of the constraints?",
        "attrs": [
            ("region", ["us-east-1", "eu-west-1", "ap-south-1", "sa-east-1"]),
            ("tier", ["free", "standard", "premium"]),
            ("encrypted", ["yes", "no"]),
            ("replicas", ["1", "2", "3", "5"]),
        ],
    },
    {
        "name": "storage volume",
        "thing": "volume",
        "question": "Which storage volume satisfies all of the requirements?",
        "attrs": [
            ("capacity", ["100g", "250g", "500g", "1t"]),
            ("media", ["ssd", "nvme", "hdd"]),
            ("snapshots", ["on", "off"]),
            ("zone", ["z1", "z2", "z3"]),
        ],
    },
    {
        "name": "service instance",
        "thing": "instance",
        "question": "Which service instance matches the requested profile?",
        "attrs": [
            ("runtime", ["node22", "py312", "go123", "jvm21"]),
            ("memory", ["512m", "1g", "2g", "4g"]),
            ("autoscale", ["on", "off"]),
            ("zone", ["a", "b", "c"]),
        ],
    },
]

_A_NAMES = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"]


def _gen_class_a(rng: random.Random, count: int, id_prefix: str = "") -> list[dict]:
    items = []
    # Round per-domain up and truncate to count so all classes balance.
    per_domain = count // len(_A_DOMAINS) + 1
    for d_idx, dom in enumerate(_A_DOMAINS):
        for k in range(per_domain):
            n_cands = rng.randint(4, 6)
            # Draw the constraints, then the satisfier, then distractors that
            # each violate at least one constraint but differ from each other.
            keep = [rng.choice(vals) for _, vals in dom["attrs"]]
            names = _A_NAMES[:n_cands]
            answer_idx = rng.randrange(n_cands)
            vectors = []
            for i in range(n_cands):
                if i == answer_idx:
                    vectors.append(list(keep))
                    continue
                while True:
                    v = [rng.choice(vals) for _, vals in dom["attrs"]]
                    violations = sum(1 for a, b in zip(v, keep) if a != b)
                    if violations >= 1 and v not in vectors:
                        vectors.append(v)
                        break
            constraints = [f"{key} must be {val}" for (key, _), val in zip(dom["attrs"], keep)]
            lines = ["Constraints: " + "; ".join(constraints) + ".", "Catalog:"]
            for name, vec in sorted(zip(names, vectors)):
                fields = ", ".join(f"{key}={val}" for (key, _), val in zip(dom["attrs"], vec))
                lines.append(f"- {dom['thing']}-{name}: {fields}.")
            candidates = [
                {
                    "id": f"{dom['thing']}-{name}",
                    "description": f"{dom['name']} {name} ("
                    + ", ".join(f"{key}={val}" for (key, _), val in zip(dom["attrs"], vec))
                    + ")",
                }
                for name, vec in sorted(zip(names, vectors))
            ]
            items.append(
                {
                    "id": f"{id_prefix}A-{d_idx:02d}{k:02d}",
                    "class": "metadata_match",
                    "context": "\n".join(lines),
                    "question": dom["question"],
                    "candidates": candidates,
                    "answer": f"{dom['thing']}-{names[answer_idx]}",
                }
            )
    return items


# ---------------------------------------------------------------------------
# Class B — lexical_semantic (hand-authored authoring table, two phrasings)
# ---------------------------------------------------------------------------

# Candidate descriptions are deliberately worded to avoid lexical overlap
# with the utterances they must attract. Each scenario contributes two
# phrasings with the same ground truth.
_B_SCENARIOS = [
    ("billing", "I think I was billed twice for the same month."),
    ("billing", "The amount taken from my card does not match my plan."),
    ("billing", "Our invoice is missing the purchase order number we asked for."),
    ("billing", "I want to switch my subscription to the annual discount."),
    ("technical", "The dashboard has shown a blank page since Tuesday."),
    ("technical", "Uploading a file makes the app crash every time."),
    ("technical", "The mobile app signs me out within a minute of logging in."),
    ("technical", "Charts stop rendering when a filter uses special characters."),
    ("shipping", "My order was supposed to arrive last week and has not."),
    ("shipping", "The tracking page has not updated in four days."),
    ("shipping", "The courier left the parcel at the wrong address."),
    ("shipping", "One item arrived damaged and the box was crushed."),
    ("account", "I cannot get back into my profile after the reset."),
    ("account", "How do I move my data to a different organization?"),
    ("account", "My display name shows the wrong company everywhere."),
    ("account", "I need to hand ownership of the workspace to a colleague."),
    ("feature", "It would help if the report could export to spreadsheets."),
    ("feature", "Can the scheduler support repeating weekly jobs?"),
    ("feature", "Please add keyboard shortcuts for the review queue."),
    ("feature", "A dark mode for late-night shifts would be great."),
    ("compliance", "Our auditors need a record of every data access."),
    ("compliance", "We must keep customer records for seven years."),
    ("compliance", "Which region keeps our data to satisfy local law?"),
    ("compliance", "We need a signed data processing agreement."),
    ("performance", "Queries slow to a crawl once the table passes a million rows."),
    ("performance", "The editor lags badly on large documents."),
    ("performance", "Batch jobs take hours where they used to take minutes."),
    ("performance", "The search box freezes for seconds before results appear."),
    ("integration", "Webhook deliveries from your service never reach our endpoint."),
    ("integration", "We need your API to push events into our warehouse."),
    ("integration", "The Salesforce connector drops custom fields on sync."),
    ("integration", "Can we authenticate with the SSO provider we already run?"),
    ("onboarding", "New hires cannot see the shared workspace on day one."),
    ("onboarding", "Where do I invite teammates and set their roles?"),
    ("onboarding", "The guided tour never appears for new sign-ups."),
    ("onboarding", "Is there a checklist for setting up our first project?"),
    ("security", "Someone tried to sign in from a country we do not operate in."),
    ("security", "We want two-step verification forced for all admins."),
    ("security", "An API key was pasted into a public ticket by mistake."),
    ("security", "Sessions stay open forever after a laptop is stolen."),
    # --- held-out pool (rows 41-80; disjoint phrasings, same domains) ---
    ("billing", "The trial converted and charged us without any notice."),
    ("billing", "Our coupon code applied the wrong discount at checkout."),
    ("billing", "We were charged in the wrong currency for this cycle."),
    ("billing", "The bank declined the renewal but the account was suspended anyway."),
    ("technical", "Push notifications arrive hours late on Android."),
    ("technical", "The CSV import silently drops rows with commas in names."),
    ("technical", "Video calls drop within five minutes on the desktop client."),
    ("technical", "Saved views reset themselves after every logout."),
    ("shipping", "Two boxes arrived but the label said three were sent."),
    ("shipping", "The driver marked our package as delivered but it never came."),
    ("shipping", "Customs is holding our parcel and we need paperwork."),
    ("shipping", "The return label printed blank and the drop-off was refused."),
    ("account", "Two teammates share one sign-in and we need separate seats."),
    ("account", "I lost access to the admin console after the domain change."),
    ("account", "Our service account was locked out overnight."),
    ("account", "Access rights changed by themselves and anyone can now delete projects."),
    ("feature", "Please let us tag conversations with custom labels."),
    ("feature", "Can the calendar sync both ways with our Exchange server?"),
    ("feature", "We would love template presets for our boards."),
    ("feature", "Add a queue view that shows only my assigned items."),
    ("compliance", "Regulators require us to prove where backups are stored."),
    ("compliance", "We need consent records attached to every marketing send."),
    ("compliance", "Our DPO asked for your sub-processor list."),
    ("compliance", "Under GDPR, PII may not leave the EU even for support."),
    ("performance", "Page loads degrade past fifty concurrent editors."),
    ("performance", "The mobile client drains battery within an hour of use."),
    ("performance", "Import speed fell by half after the last release."),
    ("performance", "Autocomplete requests time out during peak hours."),
    ("integration", "Our CRM records stop syncing after a couple of hours."),
    ("integration", "The Slack notifications fire twice for every event."),
    ("integration", "OAuth tokens expire silently and break our automation."),
    ("integration", "Your SAML metadata endpoint returns an expired certificate."),
    ("onboarding", "The sample project confuses new users more than it helps."),
    ("onboarding", "Admins have no walkthrough for configuring the first workspace."),
    ("onboarding", "Invited users land on a blank dashboard with no guidance."),
    ("onboarding", "Trial teams never discover the project templates."),
    ("security", "We detected sign-in attempts using tokens stolen from our own staff."),
    ("security", "Enable alerts whenever a service key is rotated."),
    ("security", "A former contractor may still have an active session."),
    ("security", "Brute-force attempts on our login page are not being throttled."),
]

# domain -> label -> description (no lexical overlap with utterances)
_B_DESCRIPTIONS = {
    "billing": {
        "billing": "Money and invoicing matters",
        "technical": "Software faults and defects",
        "shipping": "Delivery of physical goods",
        "account": "Credentials and workspace settings",
    },
    "technical": {
        "technical": "Product behavior that is broken",
        "billing": "Charges, refunds and invoices",
        "feature": "Requests for new capability",
        "security": "Access control and threats",
    },
    "shipping": {
        "shipping": "Parcels, couriers and delivery status",
        "technical": "Defects in the application",
        "billing": "Payment and pricing questions",
        "compliance": "Regulatory and audit needs",
    },
    "account": {
        "account": "Logins, profiles and permissions",
        "technical": "Malfunctioning features",
        "onboarding": "First-time setup and team invites",
        "billing": "Subscription and invoice issues",
    },
    "feature": {
        "feature": "Enhancement ideas and new capabilities",
        "performance": "Speed and responsiveness problems",
        "integration": "Connecting external systems",
        "technical": "Something that fails today",
    },
    "compliance": {
        "compliance": "Policy, audit and legal retention",
        "security": "Intrusion and abuse concerns",
        "account": "User and permission management",
        "feature": "Product improvement requests",
    },
    "performance": {
        "performance": "Throughput, latency and scale",
        "feature": "Missing functionality",
        "technical": "Correctness failures and errors",
        "integration": "Third-party connections",
    },
    "integration": {
        "integration": "APIs, webhooks and data exchange",
        "security": "Unauthorized access events",
        "feature": "Wishlist items for the roadmap",
        "performance": "Slow behavior under load",
    },
    "onboarding": {
        "onboarding": "Getting started, invites and roles",
        "account": "Identity and access administration",
        "feature": "Ideas for new functionality",
        "billing": "Payment and plan management",
    },
    "security": {
        "security": "Suspicious activity and protection",
        "compliance": "Audit trails and regulations",
        "account": "Profile and password help",
        "technical": "Bugs in the product",
    },
}

_B_QUESTION = "Which team should handle this message?"


def _gen_class_b(pool: slice, id_prefix: str = "") -> list[dict]:
    items = []
    scenarios = _B_SCENARIOS[pool]
    domains = sorted({s[0] for s in _B_SCENARIOS})
    d_idx = {d: i for i, d in enumerate(domains)}
    assert len(scenarios) == 40, "class B pool must carry 40 rows"
    for n, (domain, utterance) in enumerate(scenarios):
        labels = list(_B_DESCRIPTIONS[domain])
        # Fixed structural rotation of the option order per item.
        rot = (n * 2 + len(utterance)) % len(labels)
        ordered = labels[rot:] + labels[:rot]
        items.append(
            {
                "id": f"{id_prefix}B-{d_idx[domain]:02d}{n:02d}",
                "class": "lexical_semantic",
                "context": utterance,
                "question": _B_QUESTION,
                "candidates": [
                    {"id": lab, "description": _B_DESCRIPTIONS[domain][lab]} for lab in ordered
                ],
                "answer": domain,
            }
        )
    return items


# ---------------------------------------------------------------------------
# Class C — relational_compositional
# ---------------------------------------------------------------------------

_C_QUESTIONS = {
    "chain": "Fixing which single component restores every part of the chain?",
    "count": "Which region currently has the most healthy nodes?",
    "order": "Which component is brought back online first?",
}


def _gen_class_c(rng: random.Random, count: int, id_prefix: str = "") -> list[dict]:
    items = []
    # One extra per family absorbs counting-tie redraws; truncated to count.
    per_family = count // 3 + 1
    node_names = ["auth", "billing", "catalog", "dispatch", "export", "feed", "gateway", "index"]
    regions = ["north", "south", "east", "west", "central"]

    # family 0 — dependency chains: fix the single broken root of the chain.
    for k in range(per_family):
        n_nodes = rng.randint(4, 6)
        names = rng.sample(node_names, n_nodes)
        chain_len = rng.randint(2, min(3, n_nodes - 1))
        chain = [names[0]] + rng.sample(names[1:], chain_len - 1)
        # chain[-1] <- chain[-2] <- ... <- chain[0]; chain[0] is the root cause.
        broken_elsewhere = rng.choice([n for n in names if n not in chain])
        lines = []
        for dep, dependent in zip(chain[1:], chain[:-1]):
            lines.append(f"{dependent} depends on {dep}.")
        lines.append(f"{chain[0]} is failing. {broken_elsewhere} is failing.")
        rng.shuffle(lines)
        cands = names[:4]
        while chain[0] not in cands:
            cands = rng.sample(names, 4)
        items.append(
            {
                "id": f"{id_prefix}C-00{k:02d}",
                "class": "relational_compositional",
                "context": " ".join(lines),
                "question": _C_QUESTIONS["chain"],
                "candidates": [
                    {"id": c, "description": f"component {c}"} for c in sorted(cands)
                ],
                "answer": chain[0],
            }
        )

    # family 1 — counting: aggregate node health per region, pick the max.
    # Redraw on ties (deterministically, from the same rng) instead of
    # dropping the item.
    for k in range(per_family):
        while True:
            picked = rng.sample(regions, 4)
            health = {
                r: [rng.choice(["healthy", "degraded", "down"]) for _ in range(rng.randint(3, 5))]
                for r in picked
            }
            counts = {r: sum(1 for s in v if s == "healthy") for r, v in health.items()}
            best = max(counts.values())
            winners = sorted(r for r, c in counts.items() if c == best)
            if len(winners) == 1:
                break
        lines = [f"{r}: " + ", ".join(s for s in health[r]) + "." for r in picked]
        rng.shuffle(lines)
        items.append(
            {
                "id": f"{id_prefix}C-01{k:02d}",
                "class": "relational_compositional",
                "context": " ".join(lines),
                "question": _C_QUESTIONS["count"],
                "candidates": [
                    {"id": r, "description": f"region {r}"} for r in sorted(picked)
                ],
                "answer": winners[0],
            }
        )

    # family 2 — bring-up ordering: derive the first component from shuffled
    # pairwise ordering constraints (the unique node with no predecessor).
    for k in range(per_family):
        n_nodes = rng.randint(4, 5)
        names = rng.sample(node_names, n_nodes)
        lines = []
        for i in range(1, n_nodes):
            lines.append(f"{names[i]} comes back online only after {names[i - 1]}.")
        if n_nodes >= 5:
            i = rng.randrange(2, n_nodes - 1)
            lines.append(f"{names[i]} comes back online only after {names[i - 2]}.")
        rng.shuffle(lines)
        items.append(
            {
                "id": f"{id_prefix}C-02{k:02d}",
                "class": "relational_compositional",
                "context": " ".join(lines),
                "question": _C_QUESTIONS["order"],
                "candidates": [
                    {"id": c, "description": f"component {c}"} for c in sorted(names)
                ],
                "answer": names[0],
            }
        )

    # Only items with a unique computed answer survive; top up by re-walking
    # family 1 with the same rng until the requested count is met.
    valid = [i for i in items if i]
    return valid[:count]


# ---------------------------------------------------------------------------
# Assembly
# ---------------------------------------------------------------------------


def build_suite(
    seed: int = SEED,
    b_pool: slice = B_POOL_MAIN,
    suite_version: int = SUITE_VERSION,
    id_prefix: str = "",
) -> dict:
    rng = random.Random(seed)
    items: list[dict] = []
    items += _gen_class_a(rng, 40, id_prefix)[:40]
    items += _gen_class_b(b_pool, id_prefix)
    items += _gen_class_c(rng, 40, id_prefix)[:40]
    # Validate: unique ids, answers present in candidates, balanced classes.
    ids = [i["id"] for i in items]
    assert len(ids) == len(set(ids)), "duplicate item ids"
    for it in items:
        cand_ids = [c["id"] for c in it["candidates"]]
        assert it["answer"] in cand_ids, f"{it['id']}: answer not among candidates"
        assert len(cand_ids) >= 4, f"{it['id']}: too few candidates"
    classes = {}
    for it in items:
        classes[it["class"]] = classes.get(it["class"], 0) + 1
    assert all(v == 40 for v in classes.values()), f"unbalanced classes: {classes}"
    return {
        "suite_version": suite_version,
        "seed": seed,
        "instructions": INSTRUCTIONS,
        "items": items,
    }


def main() -> int:
    argv = sys.argv[1:]
    holdout = "--holdout" in argv
    argv = [a for a in argv if a != "--holdout"]
    if holdout:
        out = (
            Path(argv[0]) if argv
            else Path(__file__).parent / "suite_holdout.json"
        )
        suite = build_suite(
            seed=HOLDOUT_SEED, b_pool=B_POOL_HOLDOUT,
            suite_version=HOLDOUT_SUITE_VERSION, id_prefix="h",
        )
    else:
        out = Path(argv[0]) if argv else Path(__file__).parent / "suite.json"
        suite = build_suite()
    out.write_text(json.dumps(suite, sort_keys=True, indent=1) + "\n", encoding="utf-8")
    classes = {}
    for it in suite["items"]:
        classes[it["class"]] = classes.get(it["class"], 0) + 1
    print(f"wrote {out} ({len(suite['items'])} items: {classes})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
