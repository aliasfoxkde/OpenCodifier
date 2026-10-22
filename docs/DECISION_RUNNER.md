# The decision runner

Which process answers a model-rung decision, and why it is the fork's own
decision-only binary rather than a general-purpose serving stack.

**Status.** Measured and decided 2026-10-22. The decision record is **D44** in
[`DECISIONS.md`](DECISIONS.md); the fork's lineage is **D41** in the same file;
the build is described in [`FORK_LLAMA_SERVER.md`](FORK_LLAMA_SERVER.md).

## The question this answers

The model rung (D26) is an HTTP client to a `llama.cpp` process. The obvious
next question is whether to stand up a general-purpose inference server —
vLLM, SGLang, a combination — and point the engine at that instead.

**Answer: no.** The runner is the fork's own `llama-parallel-decision`
binary, spoken over a line protocol instead of HTTP. The measurements that
settle it are below.

## The two candidates

| | `llama-server` (HTTP) | `llama-parallel-decision` (worker) |
|---|---|---|
| Binary | 18.4 MB | **9.2 MB** |
| Transport | HTTP/1.1 JSON, `/v1/decision` | stdin/stdout, one request per line |
| Contract | takes a **schema** + contexts | takes **pre-rendered** prompt files |
| Batch | up to 256 contexts per request | one context per request |
| Speaks chat | yes (jinja templates, `/v1/chat/completions`) | no — decision only |
| Serves a model to anything else | yes | no |

Neither is a "stripped" build: both link the same `llama-decision` engine. The
size difference is the HTTP server, the chat-template machinery, the samplers,
and the OpenAI-compat surface — none of which the model rung uses.

## Why not vLLM or SGLang

Measured against this repo's constraints, not against their own benchmarks:

1. **They are Python.** This repo's binding rule is "no Python at runtime"
   (`CLAUDE.md`), and the base binary must be useful with zero ML. A vLLM
   install is tens of gigabytes of wheels and a Python interpreter; the
   decision runner here is a 9.2 MB static ELF with no runtime dependency.
2. **Their API is generation-shaped.** vLLM and SGLang expose
   `/v1/completions` and `/v1/chat/completions`: a token stream. The model
   rung needs something generation cannot give — the **full per-choice
   distribution** in one forward pass, with no sampling. `llama.cpp`'s
   decision engine scores every divergence node of the answer trie and
   returns exact constrained probabilities. A general server would have to
   reimplement that as a constrained-decoding trick, then re-derive the
   distribution from logprobs — the "constrained-decoding caution" trap
   already recorded in `RESEARCH.md` §2.
3. **They are built for throughput on GPUs.** This host has no CUDA. The
   fork's CPU build is the measured configuration; the 2B arm's p50 is
   ~600 ms, which is a CPU-serial prefill, not a GPU-batch problem.
4. **They add a dependency without adding a capability.** The engine's
   `Transport` trait (`crates/opencodifier-model/src/llamacpp.rs`) is one
   `post_json` call. A second, heavier stack would satisfy it just as well
   and cost more.

## The parity proof

The decision-only runner does **not** accept a schema — it accepts
pre-rendered prompt files. That is a real difference, and it is the thing
that has to be right or the runner is useless. So: given the same rendered
prompt, do the two runners produce the same distribution?

**They do, bit for bit.** The reconstruction is the server's own
`compile_schema` + `render_prompt` path (`tools/parallel-decision/decision-engine.cpp:610`
and `:666`):

```
server:  compile_schema(schema, instructions) -> cs.system_text
         render_prompt(tmpls, jinja, cs.system_text, context)
             -> { head, tail }        # split at the <<decision-context>> sentinel
         decide_batch(head, [tail + "{\n"], cs.inputs, opt)

runner:  static.txt  = head
         context.txt = tail + "{\n"
         field.txt   = count, values, suffix
```

Two details are load-bearing and were both wrong in the first attempt:

- The catalog quotes its allowed values (`Allowed values: "coding", "analysis",
  …`), because `compile_schema` writes `json_text(v)`, not the bare string.
  Unquoted values change the token count and the distribution.
- The prompt is rendered with the model's **jinja** chat template, not the
  plain form. The `<<decision-context>>` sentinel splits it into a cached
  head and a per-context tail. `enable_thinking: false` changes the tail to
  `<think>\n\n</think>\n\n`.

With those matched, the two runners agree exactly:

| | `coding` | `analysis` | `writing` | `research` |
|---|---|---|---|---|
| server `/v1/decision` | 0.5432648062705994 | 0.42237746715545654 | 0.012004137970507145 | 0.02235357090830803 |
| `llama-parallel-decision` | 0.543265 | 0.422377 | 0.0120041 | 0.0223536 |

The runner prints 6 significant figures; the server prints full `f64`. The
agreement is exact to the printed precision, and the token counts match
(shared 67, context 29, scored rows 5) — so the two runners fed the same
tokens are running the same engine over the same prefix.

## The measurements

Host: AMD Ryzen 7 5700U, 16 cores, 30 GB, **no CUDA**, load average ~19 at
measurement time (the host is shared — the numbers are noisy at the ±20%
level and are reported as ranges, not point estimates). Model:
`Qwen3.5-2B-Q4_K_M.gguf` (1.28 GB). Payload: one four-choice question over a
short context.

### Latency, per decision

| phase | server (HTTP) | runner (worker) |
|---|---|---|
| cold (first decision after load) | 1.42 s | 1.68–1.90 s |
| warm, steady state | 0.62–0.88 s | 0.64–0.81 s |
| process start → model loaded | 2.33 s | 1.63–1.73 s |

**The two are indistinguishable warm.** The runner's advantage is not
latency — it is the 9.2 MB binary, the absence of an HTTP surface, and the
absence of a chat/completions endpoint that could be misused as a general
LLM.

### Batching is a server feature the runner does not have

The server's `/v1/decision` takes up to 256 contexts and decodes them in one
batched pass. Measured on this host:

| contexts | wall | server `per_decision_ms` |
|---|---|---|
| 1 | 0.59 s | 584 |
| 4 | 2.21 s | 550 |
| 8 | 4.53 s | 563 |
| 16 | 9.41 s | 586 |
| 32 | 16.85 s | 525 |

**Batching bought nothing here** — per-decision time is flat, because a
2B CPU model's prefill is the cost and each context carries its own. The
batching path matters on a GPU; on this host it is a capacity feature, not
a speed feature. This is a measured result, not an assumption: the flat
`per_decision_ms` column is the finding.

### Thread count matters more than transport

The runner's worker mode takes a thread count as its last argument:

| threads | warm per decision |
|---|---|
| 6 | 0.66–0.92 s |
| **8** | **0.52–0.55 s** |
| 16 | 6.5–13.3 s (**worse than 4× slower**) |

16 threads on a 16-core shared host is catastrophic — the host is running
other work, and the runner oversubscribes. 8 threads is the setting that
matches what `llama-server` picks on its own.

## What this means for the engine

The `Transport` trait is the seam, and it is deliberately narrow:

```rust
pub trait Transport: std::fmt::Debug + Send + Sync {
    fn post_json(&self, path: &str, body: &serde_json::Value)
        -> Result<serde_json::Value, LlamaError>;
}
```

Everything testable about the model rung — payload shape, response parsing,
normalization, refusal of winner-only builds — is testable through a
scripted transport. **No live server ever runs in the test suite** (D26).

A worker-mode transport would plug in here as a second `Transport`
implementation, with the line protocol replacing the HTTP request. It is not
built yet, and the measurements say it is not urgent: the HTTP path is the
same speed, and the server's batching is the capability the runner lacks.

**What is built and measured is the fork itself**, and the runner's
availability is now a fact rather than a proposal.

## Reproducing the parity check

The reconstruction is scripted so it can be re-run when the fork's rendering
changes:

```bash
# 1. the server renders the prompt it would use
curl -s -X POST http://127.0.0.1:8097/apply-template \
  -H 'Content-Type: application/json' --data-binary @body.json -o tmpl.json

# 2. split at the sentinel; write head -> static.txt, tail -> context.txt
#    (the field file is: count, one allowed value per line, then the suffix)

# 3. the runner answers the same rendered prompt
./llama-parallel-decision model.gguf 0 @shared static.txt context.txt field.txt
```

`apply-template` is the server's own renderer, so this cannot drift from
the server by construction — as long as the field file is built the way
`compile_schema` builds it (quoted values, `"  \"choice\": "` suffix).

## Honest limits

- The measurements are **CPU-only on a shared host**. No CUDA on this
  machine, so the GPU batch numbers are not measured, only reasoned about
  from the flat `per_decision_ms` column.
- The load average was ~19 during measurement. The ±20% spread in the
  latency tables is that, not the runner.
- The 4-choice/short-context payload is small. A wide question (many
  choices) or a long context shifts the prefill/scoring split, and the
  tables would need re-measuring.
- The worker protocol is **not** yet implemented on the Rust side. Nothing
  in the engine depends on it.
- The `woct0rdho/llama.cpp` `strix` branch (fused LoRA kernels) is still
  unfetchable. It is irrelevant to this decision — it is a LoRA-throughput
  lever, not a serving-stack one — but it is still unmeasured.
