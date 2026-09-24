# Fixtures

Hand-written wire payloads that lock each adapter's on-the-wire shape
(`PLANNING.md` §42, `DECISIONS.md` D10). Every file here is loaded by
`crates/opencodifier-schema/tests/fixtures.rs`: the test named in the table
below decodes it, re-encodes it, and compares against the file, so a change
to any wire shape fails a test named after the fixture it broke.

Fixtures are inputs, not examples: they describe payloads OpenCodifier must
accept (foreign formats) or produce (native), and each one is the reviewable
record of a projection decision.

## Conventions

- One fixture per question kind per format: `choice`, `score`, `boolean`.
  The Jev fixtures come in request/response pairs because that format has
  two distinct documents.
- Foreign payloads deliberately carry extra vendor fields (`usage`, `model`,
  `tool_choice`) that OpenCodifier ignores. `native` is the exception: it
  rejects unknown fields (`ARCHITECTURE.md` §7).
- Names are `<format>/<what>.json`; the extension states the role the
  document plays (`.format.json` for an OpenAI structured-output `format`,
  `.tool.json` for an Anthropic tool definition, plain `.json` otherwise).

## Catalogue

| Fixture | What it locks | Locking test |
| --- | --- | --- |
| `native/request.json` | Canonical request: state + fact, one question of each kind, policy and metadata. Identity projection — byte-exact both ways. | `fixture_native_request_round_trips` |
| `native/response.json` | Canonical response: choice/score/boolean answers with full distributions, `accept` outcome, confidence report, trace entry, metrics. | `fixture_native_response_round_trips` |
| `openai/choice-enum.format.json` | Strict-mode `format` for a choice question: `enum` + `additionalProperties: false` + every field in `required`; candidate descriptions ride in the property `description` marker. | `fixture_openai_choice_enum_format_round_trips` |
| `openai/score-bounded-integer.format.json` | Strict-mode `format` for a score question: bounded `integer` whose level order is carried by the `opencodifier:levels=` marker. | `fixture_openai_score_bounded_integer_format_round_trips` |
| `openai/boolean.format.json` | Strict-mode `format` for a boolean question: `type: "boolean"`, no enum, no bounds. | `fixture_openai_boolean_format_round_trips` |
| `anthropic/tool-choice.tool.json` | Anthropic tool definition with two choice properties; byte-locks the sorted property/`required` order the encoder emits. | `fixture_anthropic_tool_choice_round_trips` |
| `anthropic/tool-score.tool.json` | Anthropic tool definition for a score question, same integer + level-marker mapping as OpenAI. | `fixture_anthropic_tool_score_round_trips` |
| `anthropic/tool-boolean.tool.json` | Anthropic tool definition for a boolean question. | `fixture_anthropic_tool_boolean_round_trips` |
| `jev/systemone-choice-request.json` | Jev request with a `choice` question: `criteria` label → description, plus the `model` fact that becomes state metadata. | `fixture_jev_systemone_choice_request_round_trips` |
| `jev/systemone-choice-response.json` | Jev response whose `answers` map carries the chosen label only; `usage` is ignored rather than invented. | `fixture_jev_systemone_choice_response_round_trips` |
| `jev/systemone-score-request.json` | Jev request with a `score` question whose `criteria` are deliberately **not** in sorted key order — document order is the level order. | `fixture_jev_systemone_score_request_round_trips` |
| `jev/systemone-score-response.json` | Jev score answer as a probability map over level indices `"0".."3";` the weighted mean (1.91) selects level `moderate`. | `fixture_jev_systemone_score_response_round_trips` |
| `jev/systemone-noul-request.json` | Jev request with a `noul` (boolean) question: no `criteria`, just instructions. | `fixture_jev_systemone_noul_request_round_trips` |
| `jev/systemone-noul-response.json` | Jev `noul` answer as a bare probability; `0.87 >= 0.5` decides `true`. | `fixture_jev_systemone_noul_response_round_trips` |

## Adding a fixture

1. Add the file under the format's directory, following the naming rules
   above, and keep it minimal — one behaviour per fixture.
2. Add a `fixture_<name>_round_trips` test to
   `crates/opencodifier-schema/tests/fixtures.rs` that decodes, re-encodes,
   and asserts against the file.
3. Add a row to this table. `readme_names_every_fixture` fails if a file on
   disk is missing from the table, or the table names a file that does not
   exist.
