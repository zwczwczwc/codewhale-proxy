# Responses Prefix-Cache Bypass Validation Report

- Date: 2026-08-05 (live command output)
- Source: `/root/projects/codewhale-proxy/source`, branch `feat/gpt-responses-transport`
- Result: **CONDITIONAL**
- No source code or production configuration was modified.

## 1. Scope and safety

The existing production listener was checked before and after the run:

```text
before: 0.0.0.0:11441 LISTEN, cc-proxy pid 3603271
health: GET http://127.0.0.1:11441/health -> HTTP 200
```

A temporary instance was started only on `127.0.0.1:11449` with:

```text
LISTEN_ADDR=127.0.0.1:11449
ESWITCH_URL=http://clawbot:11434
MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml
RUST_LOG=info
```

Startup output confirmed:

```text
Loaded 5 model profiles, 4 providers
Listening on: 127.0.0.1:11449
eswitch health check: OK
Server ready on 127.0.0.1:11449
```

`getent hosts clawbot` returned `100.64.0.1 clawbot.hermes.tailnet clawbot`. All test requests used the placeholder header `Authorization: Bearer validation-placeholder`; the value was not logged or written to this report.

After testing, the temporary process was terminated. Final state:

```text
11449: no listener
19091: no listener (local mock helper also cleaned)
11441: still LISTEN by existing cc-proxy pid 3603271
GET http://127.0.0.1:11441/health -> HTTP 200
```

The production 11441 service was not restarted.

## 2. Local test baseline

The current implementation was exercised with the repository test suite:

```text
cargo test --locked
76 passed, 0 failed
```

The relevant implementation tests passed, including:

- `relocate_on_does_not_mutate_previous_history_items`
- `relocated_responses_wire_keeps_three_round_history_stable`
- `tool_result_is_function_call_output_and_arguments_are_json`
- `three_hashes_have_independent_canonical_semantics`

These are local unit tests, not a substitute for the live upstream results below.

## 3. Live bypass results through clawbot

All requests below traversed the temporary `127.0.0.1:11449` instance to `http://clawbot:11434`.

### 3.1 Fixed ordinary request

```text
POST /v1/messages -> HTTP 200
content-type: application/json
stop_reason: max_tokens
input_tokens: 675
output_tokens: 32
cache_read_input_tokens: 0
cache_creation_input_tokens: 0
```

The request reached the Responses-backed path successfully, but the upstream response produced no final content blocks before `max_tokens`; this is recorded as protocol-level success with output truncation, not as a successful normal-text assertion.

A second fixed-history request returned:

```text
POST /v1/messages -> HTTP 200
input_tokens: 674
output_tokens: 32
stop_reason: max_tokens
```

### 3.2 Multi-round history

The first round with `U1` completed with HTTP 200. The follow-up containing `U1 + A1 + U2` returned:

```text
HTTP 502
Responses API error (400 Bad Request): Invalid value: 'input_text'.
Supported values are: 'output_text' and 'refusal'.
```

Therefore, a live multi-round cache-hit claim is **not proven** by this run. The 502 is separately recorded as an upstream/request-shape failure and must not be mislabeled as a cache miss.

### 3.3 Tool continuation

The tool-continuation attempt did not pass:

```text
follow-up: client TimeoutError after 20.02s
retry follow-up: HTTP 502
Responses API error (400 Bad Request): No tool call found for function call output with call_id placeholder.
```

The placeholder call ID was intentionally not a real upstream tool-call ID. This result proves the path rejects invalid continuation state; it does not prove valid tool continuation behavior.

### 3.4 Streaming

```text
POST /v1/messages with stream=true -> HTTP 200
content-type: text/event-stream
bytes: 432
```

The response contained Anthropic SSE framing beginning with `message_start`. The test client did not parse the full event sequence into a pass assertion, so streaming is **conditionally observed**, not fully accepted.

### 3.5 Direct Responses route / fallback check

```text
POST /v1/responses -> HTTP 404
```

The temporary cc-proxy exposes the Anthropic `/v1/messages` compatibility route; it did not silently turn a direct `/v1/responses` request into Chat Completions. This is consistent with “Responses does not fallback” for the tested route, while direct `/v1/responses` is not an exposed public route in this binary.

## 4. Cache observations

The temporary process emitted one stable Responses fingerprint for the tested requests:

```text
Responses request built prefix_fingerprint=e54b4477a8e3e008 model=gpt-5.6-luna
```

Live usage fields were preserved as:

```text
cache_read_input_tokens
cache_creation_input_tokens
```

Observed values for the completed requests were zero for both fields. No cache hit was established in this run. This is not sufficient evidence that cache is unavailable: the live run also encountered upstream 502 and timeout behavior, and the ordinary test vector was not identical to the long-prefix vectors in prior reports.

The implementation's local tests do establish the intended structural property: relocation is appended as a synthetic tail and does not mutate prior history items across three rounds. That property is source/unit-test evidence, not live cache-hit evidence.

## 5. Failure classification

| Observation | Classification | Evidence |
|---|---|---|
| Completed request with `cache_read_input_tokens=0` | cache miss / no hit observed | HTTP 200 usage fields |
| Multi-round follow-up | upstream/request-shape failure | HTTP 502 wrapping upstream HTTP 400 `input_text` rejection |
| Tool continuation retry | invalid continuation failure | HTTP 502 wrapping missing `call_id` |
| Tool continuation first attempt | timeout | client `TimeoutError` after 20.02s |
| Streaming | partial pass | HTTP 200 SSE, 432 bytes |
| Direct `/v1/responses` | no exposed route, no fallback observed | HTTP 404 |

Per requirement, the 502/timeout results are not counted as cache misses.

## 6. Final conclusion

**CONDITIONAL.** The production listener on 11441 remained healthy and was not restarted. The temporary 11449 → clawbot path started successfully, used a placeholder Authorization header without recording its value, and was cleaned up successfully. Local validation is green at 76/76 tests and includes three-round relocation/history stability tests. Live ordinary and SSE requests reached the Responses-backed path, but this run did not establish a cache hit; valid tool continuation and complete multi-round live validation remain unproven because the run encountered timeout/502 responses and an upstream request-shape rejection. No production deployment approval should be inferred from this report.
