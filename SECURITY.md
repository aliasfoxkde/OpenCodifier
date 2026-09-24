# Security Policy

## Supported versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | yes       |

## Reporting a vulnerability

OpenCodifier is designed to run fully local and offline, but it still parses
hostile input: decision schemas, candidate descriptions, graph definitions,
and model manifests (PLANNING.md §66).

Please report vulnerabilities privately via GitHub
[security advisories](https://github.com/aliasfoxkde/OpenCodifier/security/advisories/new)
rather than public issues. Include a minimal reproducer (input file +
invocation) and the affected crate/version.

You will receive an initial response within 7 days. Coordinated disclosure:
we aim to release a fix within 90 days of a confirmed report, and will credit
reporters in the release notes unless asked not to.

## Security-relevant invariants

Contributors must preserve these properties; violations are security bugs:

1. **No telemetry, no cloud calls.** The runtime must not open network
   connections except when the user explicitly installs a model or starts a
   server with an explicit host flag.
2. **Data is not executable.** Text from state, candidate descriptions, tool
   descriptions, and schemas must never modify policy, thresholds, graph
   structure, filesystem paths, or tool permissions.
3. **Input is hostile.** Schema and graph parsers must enforce resource
   limits (max input bytes, nodes, candidates, questions, execution time)
   and must not panic on malformed input.
4. **Model integrity.** Model downloads are explicit and verified against
   the SHA-256 in a signed manifest before use.
5. **Local bind default.** `opencodifier serve` binds `127.0.0.1` unless the
   operator passes an explicit non-local host flag.
