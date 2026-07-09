# S1c Migration Map

Date: 2026-07-09
Nature: docs-only migration map after S1b. This is not a runtime expansion plan and does not change code paths.

## Summary

S1b has landed the first Rust gateway slice on `main`: the formal proxy can launch the Rust sidecar only when `CSSWITCH_GATEWAY=rust`, the effective adapter is `deepseek`, and `shim_mode=off`. Python remains the default and remains required for all other formal proxy adapters plus temporary probe paths.

S1c turns the post-S1b state into an executable migration map. It separates formal runtime proxy coverage, temporary Python probe surfaces, catalog-only diagnostics, and future PR slices. The goal is to make the next implementation stage small and reviewable instead of treating "remove Python" as one rewrite.

This document is public-safe. It records source-level behavior, tests, and packaging facts only. It does not claim CI green, GUI E2E, live provider success, real `~/.claude-science` behavior, real token behavior, Developer ID signing, notarization, Gatekeeper, or full packaged app runtime verification.

## Current Surface

### Formal Runtime Proxy

| Adapter family | Current production path | Rust status | Python status | Migration blocker |
|---|---|---|---|---|
| `deepseek` + `shim_mode=off` | Formal proxy launched by `start_proxy_for` | Implemented as opt-in sidecar via `CSSWITCH_GATEWAY=rust` | Still default fallback | Parity hardening for error mapping, streaming, CONNECT, and packaged runtime confidence before default flip |
| `deepseek` + `shim_mode=detect/rewrite` | Python proxy with DSML shim behavior | Not eligible | Required | DSML detection/rewrite parser and stream rewrite semantics are still Python-only |
| `qwen` | Python OpenAI Chat translation path | Not eligible | Required | Anthropic Messages to OpenAI Chat transform, streaming/tool-call mapping, model policy, and DashScope fixed endpoint behavior need Rust contracts first |
| `relay` | Python Anthropic-compatible passthrough | Not eligible | Required | Force model shell, relay thinking policy, tool schema normalization, Kimi web-search filter, CONNECT expectations, and provider-specific compatibility rules need separate contracts |
| `openai-custom` | Python OpenAI Chat custom endpoint path | Not eligible | Required | Editable base URL, model pinning, OpenAI auth, model discovery, and Chat transform contracts need Rust feasibility tests |
| `openai-responses` | Python OpenAI Responses custom endpoint path | Not eligible | Required | Responses request/response mapping, function tools, conservative output caps, and provider quirks need Rust feasibility tests |

### Temporary Python Probe Paths

These paths are not part of the S1b formal proxy claim and must not be described as Rust-covered:

| Surface | Current entry | Current dependency | Migration blocker |
|---|---|---|---|
| Candidate validation | `scratch_validate_candidate` in profile switching | Temporary `python3 proxy/csswitch_proxy.py` plus scratch port/secret | Needs a Rust scratch probe abstraction that preserves transaction semantics and ambiguous/auth/model-error classification |
| Active profile transaction probing | `set_active_profile_txn` scratch phase | Temporary Python proxy before formal proxy commit | Needs Rust candidate probing before replacing Python in profile transactions |
| Model discovery | `fetch_models` / `runtime/model_discovery.rs` | Temporary Python proxy and `/v1/models` scratch | Needs Rust discovery mode that never pins formal launch env such as `CSSWITCH_RELAY_MODEL` or `CSSWITCH_OPENAI_MODEL` |
| Probe classification | `scratch.rs` | Shared Rust classifier around Python child process results | Needs reusable Rust transport/probe runner before deleting Python scripts |

### Python Compatibility Modules

| Module | Current role | Migration note |
|---|---|---|
| `proxy/csswitch_proxy.py` | Formal Python proxy entrypoint for all non-Rust-eligible adapters and scratch probes | Cannot be removed until every formal and temporary path has Rust equivalents |
| `proxy/provider_policy.py` | Model mapping, max token clamp, thinking/tool-choice policy | Can migrate rule by rule into gateway contracts |
| `proxy/anthropic_compat.py` | Anthropic passthrough normalization, relay/Kimi tool handling, stream rewrite helpers | Relay and DSML slices depend on this behavior |
| `proxy/openai_chat_compat.py` | Anthropic to OpenAI Chat translation | Qwen and OpenAI custom feasibility depend on this |
| `proxy/responses_compat.py` | Anthropic to OpenAI Responses translation | Responses feasibility depends on this |
| `proxy/dsml_shim.py` | Optional DSML leak detection/rewrite | DeepSeek detect/rewrite must stay Python until this has Rust contracts |
| `proxy/model_discovery.py` | Normalized `/v1/models` responses, shell model fallback, live/builtin/manual semantics | Must stay separate from formal launch model pinning |
| `proxy/http_transport.py` | Shared HTTP behavior for Python proxy internals | Future Rust transport should copy contracts, not import Python |

## Catalog To Runtime Map

The catalog remains a read-only fact and diagnostics source in S1c. It is not a live-provider certificate and it does not drive runtime behavior yet.

| Rule group | Examples | S1c class | Runtime implication |
|---|---|---|---|
| Runtime-portable provider/tool policy | `provider.deepseek.anthropic-native`, `tool.deepseek.forced-tool-choice-disable-thinking`, `tool.relay.input-schema-normalize`, `tool.kimi.web_search.server-tool-filter`, `provider.dashscope.responses-tools-cap` | Future runtime contract candidates | Can become Rust gateway tests and pure functions, slice by slice |
| Diagnostics-only boundaries | `science.version.*`, `science.auth.*`, `transport.http-proxy.not-set-by-default`, `transport.connect.*` | Diagnostics and docs | Keep surfaced through status/catalog; do not treat as provider support |
| Unsupported or unknown hosted capabilities | Anthropic-hosted HCLS MCP, Directory connectors, official remote skills, local/GitHub skill discoverability before verification | Boundary or discovery backlog | Do not enter Rust gateway migration; require separate product probes or local alternative workflows |
| Future transport gap | `transport.upstream-proxy.planned-for-http-mcp`, external Streamable HTTP MCP | Separate transport project | Do not bundle into provider runtime migration |

S1c does not change `catalog/capabilities.v1.json` schema. If future implementation slices need runtime-driven rules, they should first add tests that prove equivalence to existing Python behavior, then add a narrow Rust-side representation derived from the catalog or from the same source facts.

## Implementation Slices

### Slice A: DeepSeek Rust Parity Hardening

Goal: harden the existing Rust sidecar without widening eligibility.

- Target: `deepseek + shim off` formal proxy only.
- Non-goals: qwen, relay, OpenAI custom, DSML detect/rewrite, scratch/model discovery migration, default flip.
- Work items: compare Python and Rust behavior for nonstream errors, stream errors, CONNECT blocked/direct tunnel, malformed requests, auth failure JSON, and `/v1/models` shell body.
- Required tests: gateway crate unit tests, `test.test_gateway_rust`, targeted Python-vs-Rust loopback vectors, `cargo test --manifest-path desktop/src-tauri/Cargo.toml`, `cargo test` in `desktop/gateway`, `bash test/run-contract.sh`.
- Review gate: prove `gateway_kind` and `shim_mode` still force restart on identity changes.
- Claim boundary: may say parity hardening passed locally; must not say default Rust, Python-free runtime, GUI E2E, CI green, live provider, signing/notarization, or Gatekeeper.
- Rollback: leave `CSSWITCH_GATEWAY` unset or set non-`rust`; Python remains default.

### Slice B: Qwen Rust Feasibility

Goal: build contracts for Qwen's OpenAI Chat translation before production routing.

- Target: transform and response mapping tests for Qwen-compatible OpenAI Chat behavior.
- Non-goals: production Rust eligibility, custom OpenAI, Responses, relay, live DashScope calls.
- Work items: create fixtures for Anthropic Messages input, OpenAI Chat outbound body, tool-call mapping back to Anthropic shape, streaming deltas, max token/model policy.
- Required tests: Rust pure transform tests or fixture parser tests, Python parity tests against `openai_chat_compat.py`, existing proxy unit tests.
- Review gate: no new `gateway_kind_for` eligibility for `qwen` until fixtures cover nonstream, stream, tools, and model policy.
- Claim boundary: feasibility only; no runtime switch.
- Rollback: remove the feasibility fixture/tests without affecting production routing.

### Slice C: Relay Rust Feasibility

Goal: split relay behavior into testable contracts before any Rust runtime expansion.

- Target: Anthropic-compatible relay passthrough semantics.
- Non-goals: production Rust eligibility, upstream proxy, MCP transport, provider live probes.
- Work items: contract tables for force model shell, `CSSWITCH_RELAY_MODEL`, relay thinking policy, loose tool schema normalization, Kimi `web_search` filtering, SiliconFlow forced named tool-choice downgrade, `/v1/models` live/builtin/manual semantics.
- Required tests: Python fixture parity for request transform and stream rewriter behavior, catalog rule coverage tests for affected provider/tool rules, loopback-only relay mock tests.
- Review gate: prove model discovery scratch does not pin formal runtime model env.
- Claim boundary: relay feasibility only; no hosted capability or external MCP claim.
- Rollback: keep all relay profiles on Python.

### Slice D: OpenAI Custom And Responses Feasibility

Goal: prepare Rust contracts for custom OpenAI Chat and OpenAI Responses families.

- Target: `openai-custom` and `openai-responses` adapter behavior.
- Non-goals: production Rust eligibility, qwen default changes, relay behavior, live providers.
- Work items: contract fixtures for editable base URL, bearer auth, `/models` discovery, model pinning via `CSSWITCH_OPENAI_MODEL`, Chat Completions mapping, Responses mapping, function tools, DashScope Responses caps and `web_search` drop.
- Required tests: parity against `openai_chat_compat.py` and `responses_compat.py`, `test_proxy_auth` model discovery cases, no live provider tests.
- Review gate: reject Anthropic-looking base URLs for OpenAI custom remains intact.
- Claim boundary: feasibility only; no guarantee any custom endpoint works live.
- Rollback: keep both adapters on Python.

### Slice E: Temporary Python Path Replacement Plan

Goal: design the final Python removal prerequisites after formal proxy slices mature.

- Target: scratch validation, model discovery, profile transaction probing, and probe classification.
- Non-goals: immediate deletion of `proxy/*.py`, changing profile transaction semantics, changing active profile commit rules.
- Work items: define Rust probe runner, scratch port/secret lifecycle, typed outcomes, no-formal-state-mutation rule, model discovery response contract, and cleanup behavior.
- Required tests: Rust unit tests for probe outcome classification, loopback scratch probes, transaction tests for commit/rollback/abort, `bash test/run-contract.sh`, and full `bash test/run_all.sh` when environment allows.
- Review gate: prove temporary probes do not stop or mutate the serving proxy unless the transaction explicitly commits.
- Claim boundary: only after Slice E implementation and all formal slices are complete can Python removal be discussed.
- Rollback: keep Python scratch path and formal Python fallback.

## Validation Gates

### S1c Docs-Only Gate

Run for this planning stage:

```bash
git diff --check
python3 -m unittest test.test_capability_catalog test.test_proxy_packaging -v
```

These gates validate documentation cleanliness plus static catalog/packaging smoke coverage. They do not prove runtime behavior.

### Future Implementation Gate

Use the narrowest gate that matches the slice, then broaden before merge:

- Rust gateway changes: `cargo test` in `desktop/gateway`, plus targeted Python loopback gateway tests.
- Tauri launch/identity changes: `cargo test --manifest-path desktop/src-tauri/Cargo.toml`.
- Python parity or compatibility changes: relevant `python3 -m unittest test.test_provider_policy test.test_anthropic_compat test.test_proxy_units test.test_proxy_auth -v`.
- Contract closeout: `bash test/run-contract.sh`.
- Release-readiness candidate: `bash test/run_all.sh --require-release-ready` only in an environment where all layers are available.

### Claim Language

- `current-env clean`: local S0 runner found no failing layer; env-blocked can still exist.
- `release-ready green`: all S0 layers passed with no env-blocked result.
- `packaged smoke`: app/bundle file layout or local bundle validation only.
- `live provider verified`: requires explicit live-provider run with user-approved credentials; not part of S1c.
- `CI green`: requires GitHub checks; local gates are not CI.
- `GUI E2E`: requires actual GUI flow automation or manual evidence; not implied by backend tests.
- `signing/notarization/Gatekeeper`: requires dedicated Apple signing/notarization/quarantine validation; ad hoc local codesign is not enough.

Each future implementation slice should end with a read-only review pass that checks scope boundaries, catalog overclaims, test coverage, and whether the slice accidentally expanded runtime eligibility.

## Next Recommended Execution

After S1c, start with Slice A. It is the safest next implementation PR because it hardens the already-merged Rust sidecar without expanding provider surface. Qwen, relay, OpenAI custom, Responses, upstream proxy, and MCP transport should remain feasibility or planning work until Slice A's parity evidence is stronger.

## Assumptions

- This stage is docs-only and does not change public runtime APIs, Tauri command schemas, catalog schema, proxy wire behavior, or UI.
- The S1b packaged smoke checkpoint is part of this stage's baseline record.
- No real `~/.claude-science`, `.env`, token, live provider, or port `8765` is used.
- MCP, skills, hosted capabilities, and external Streamable HTTP MCP remain catalog/diagnostics boundaries unless a later product-specific phase proves otherwise.
