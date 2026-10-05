#!/usr/bin/env python3
"""Render release notes from a template plus the staged build attestations.

The mechanical slots (`@@NAME@@`) are filled only from evidence: the
attestations under `dist/` carry the digests, sizes, targets, toolchain
versions, and dependency-tree hashes, and the release notes quote them. The
slots a human owes the release page — highlights, gate receipts, per-artifact
verification level — are left as `<filled-by-human>` markers, because no
script here can measure what changed and why it matters or whether a gate
actually ran.

Refusals that matter: attestations from different commits, toolchains, or
lockfiles are a mixed release and are rejected rather than averaged into one
document; an unfilled `@@SLOT@@` in the output is a failure, not a published
blank; and an existing output file is never overwritten without `--force`.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

# Sibling module, not a package import: the release tools live flat in
# scripts/ beside the other single-purpose scripts.
from generate_attestation import (
    SCHEMA_VERSION,
    AttestationError,
    load_attestation,
    run,
)

SLOT = re.compile(r"@@[A-Z_]+@@")
COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)


ISO_INSTANT = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$")


def canonical(value: object) -> object:
    """Normalize a value for comparison across attestations.

    Timestamps are compared as instants, not as strings: `…T17:47:47Z` and
    `…T12:47:47-05:00` are the same moment, and an attestation writer that
    spells UTC differently is not a mixed release.
    """
    if isinstance(value, str) and ISO_INSTANT.match(value):
        return dt.datetime.fromisoformat(value).astimezone(dt.timezone.utc).isoformat()
    return value


def one_value(name: str, documents: list[dict[str, object]], getter) -> object:
    """Read one field that must be identical across every attestation."""
    values: dict[str, object] = {}
    for document in documents:
        raw = getter(document)
        values[json.dumps(canonical(raw), sort_keys=True)] = raw
    if len(values) != 1:
        raise AttestationError(
            f"{name} differs across attestations ({len(values)} distinct values); "
            "attestations from different builds are not one release"
        )
    return next(iter(values.values()))


def linkage(magic: str) -> str:
    """Static/dynamic statement for the artifact table, from the file magic."""
    if "static-pie linked" in magic or "statically linked" in magic:
        return "static"
    interpreter = re.search(r"interpreter ([^,]+)", magic)
    if interpreter:
        return f"dynamic (interpreter {interpreter.group(1)})"
    return "unknown linkage"


def artifact_table(documents: list[dict[str, object]]) -> str:
    """One markdown row per attested artifact, sorted by target for stability."""
    rows = [
        "| Target | Artifact | Bytes | SHA-256 | Linkage | Attestation |",
        "|--------|----------|------:|---------|---------|-------------|",
    ]
    ordered = sorted(documents, key=lambda d: str(d["artifact"]["target"]))
    for document in ordered:
        artifact = document["artifact"]
        rows.append(
            f"| `{artifact['target']}` | `{artifact['name']}` "
            f"| {artifact['bytes']} | `{artifact['sha256']}` "
            f"| {linkage(str(artifact['file_magic']))} "
            f"| `{artifact['path']}.attestation.json` |"
        )
    return "\n".join(rows)


def build_commands(documents: list[dict[str, object]]) -> str:
    """The recorded build command per target, as a runnable block."""
    lines = ["```bash"]
    for document in sorted(documents, key=lambda d: str(d["artifact"]["target"])):
        target = document["artifact"]["target"]
        lines.append(f"# {target}")
        lines.append(str(document["build"]["build_command"]))
    lines.append("```")
    return "\n".join(lines)


def feature_flags(documents: list[dict[str, object]]) -> str:
    """The feature set the artifacts were built with (identical across all)."""
    features = one_value("feature flags", documents, lambda d: d["build"]["features"])
    no_default = one_value(
        "no_default_features", documents, lambda d: d["build"]["no_default_features"]
    )
    text = ", ".join(features) if features else "none"
    return text + (", default features disabled" if no_default else "")


def workspace_version(repo: pathlib.Path) -> str:
    """Version of the released crate, from cargo's own view of the workspace."""
    metadata = json.loads(
        run(["cargo", "metadata", "--no-deps", "--format-version", "1"], repo)
    )
    for package in metadata["packages"]:
        if package["name"] == "opencodifier-cli":
            return str(package["version"])
    raise AttestationError("opencodifier-cli is not a workspace package — cannot date a release")


def previous_tag(repo: pathlib.Path) -> str:
    """The tag the release is measured against, for the highlights diff."""
    return run(["git", "describe", "--tags", "--abbrev=0"], repo)


def fill(template: str, slots: dict[str, str]) -> str:
    """Substitute every known slot; fail on any slot left standing.

    The leftover check runs over the rendered text with its HTML comments
    removed: a comment may legitimately show a slot to explain the convention,
    while prose may not ship one.
    """
    text = template
    for name, value in slots.items():
        text = text.replace(f"@@{name}@@", value)
    leftover = sorted(set(SLOT.findall(COMMENT.sub("", text))))
    if leftover:
        raise AttestationError(
            f"template slots the attestations cannot fill: {', '.join(leftover)}"
        )
    return text


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Render release notes from the template plus build attestations."
    )
    parser.add_argument(
        "--template",
        type=pathlib.Path,
        default=pathlib.Path("docs/RELEASE_NOTES_TEMPLATE.md"),
        help="release notes template (default: docs/RELEASE_NOTES_TEMPLATE.md)",
    )
    parser.add_argument(
        "--output",
        type=pathlib.Path,
        default=pathlib.Path("dist/RELEASE_NOTES.md"),
        help="where to write the rendered notes (default: dist/RELEASE_NOTES.md)",
    )
    parser.add_argument(
        "--attestation",
        type=pathlib.Path,
        action="append",
        required=True,
        help="an attestation to quote (repeat for every artifact)",
    )
    parser.add_argument(
        "--repo-root",
        type=pathlib.Path,
        default=pathlib.Path(__file__).resolve().parent.parent,
        help="repository root the release is cut from",
    )
    parser.add_argument(
        "--tag", default="", help="release tag (default: v<crate version>)"
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="overwrite an existing release-notes file",
    )
    args = parser.parse_args()

    try:
        repo = args.repo_root.resolve()
        if args.output.exists() and not args.force:
            raise AttestationError(
                f"{args.output} already exists — pass --force to rewrite it"
            )
        documents = [load_attestation(path) for path in args.attestation]
        if not documents:
            raise AttestationError("no attestations given; nothing to render")

        version = workspace_version(repo)
        head = str(one_value("commit", documents, lambda d: d["source"]["commit"]))
        base_tag = previous_tag(repo)
        slots = {
            "TAG": args.tag or f"v{version}",
            "DATE": dt.datetime.now(dt.timezone.utc).date().isoformat(),
            "COMMIT": head,
            "COMMIT_SHORT": head[:12],
            "COMMIT_DATE": str(
                one_value("commit date", documents, lambda d: d["source"]["commit_date_utc"])
            ),
            "PREVIOUS_TAG": base_tag,
            "RUSTC": str(one_value("rustc", documents, lambda d: d["build"]["rustc"])),
            "CARGO": str(one_value("cargo", documents, lambda d: d["build"]["cargo"])),
            "CARGO_LOCK_SHA256": str(
                one_value(
                    "Cargo.lock digest", documents, lambda d: d["build"]["cargo_lock_sha256"]
                )
            ),
            "CARGO_TREE_SHA256": str(
                one_value(
                    "workspace tree digest",
                    documents,
                    lambda d: d["build"]["cargo_tree_sha256"],
                )
            ),
            "FEATURES": feature_flags(documents),
            "SCHEMA_VERSION": SCHEMA_VERSION,
            "ARTIFACT_TABLE": artifact_table(documents),
            "BUILD_COMMANDS": build_commands(documents),
            "HIGHLIGHTS": (
                "<filled-by-human> — one bullet per user-visible change since "
                f"{base_tag}, each citing its DECISIONS.md entry or "
                "CHANGELOG.md section."
            ),
        }
        rendered = fill(args.template.read_text(encoding="utf-8"), slots)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    except AttestationError as error:
        print(f"FAIL  {error}", file=sys.stderr)
        return 1

    print(
        f"PASS  {args.output}\n"
        f"      {len(documents)} artifact(s) quoted, "
        f"commit {slots['COMMIT_SHORT']}, tag {slots['TAG']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
