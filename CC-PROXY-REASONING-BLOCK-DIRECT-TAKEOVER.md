# cc-proxy Responses reasoning block — direct takeover plan

## Goal
在不修改生产 11441、cc-connect 配置、Chat 路径或 Responses input/history/tool wire 的前提下，使 gpt-5.6-luna Responses 的可读 reasoning summary 进入标准 Anthropic `thinking` block，并保持工具调用、terminal、cache usage 与前缀语义。

## Current evidence
- `t_c814a8ce` 已 blocked：源码静态质量通过；真实 A/B 有 timeout、incomplete、A/B cache 分区差异和 continuation 不稳定，不能当作 fully verified。
- 真实直连 `clawbot:11434/v1/responses` 已验证：`reasoning.effort=max` 仅返回 reasoning item 空 summary；增加 `reasoning.summary=auto` 或 `detailed` 后返回 `response.reasoning_summary_text.delta/done`，非流式 output reasoning item 含 summary text。
- 当前 cc-connect `thinking_messages=true`，且 v1.4.1 已有 `Claude thinking -> EventThinking -> Feishu` 路由；不修改该层。

## Implementation slices (TDD)
1. Add GPT Responses summary mode to provider config/request. Default GPT Responses to `auto`; explicit `off` remains possible. Keep summary mode out of cache identity hash; keep `instructions/input/tools/tool_choice/history` byte semantics unchanged.
2. Add RED/GREEN request tests: serialized wire contains `reasoning.summary=auto`; comparing summary off/auto keeps input/history/wire hashes and input identical.
3. Harden non-stream response status handling: readable summary only; no encrypted/redacted fabrication; reject failed/cancelled/non-terminal statuses; ignore empty summary text.
4. Harden SSE state machine: key reasoning accumulators by `item_id + summary_index`, handle real summary delta/done fields, preserve text/reasoning/tool block lifecycle, nested failed error, cancelled and abnormal EOF as errors, and closed-tool late delta policy.
5. Harden out-of-order tool metadata without emitting an item_id as final Anthropic tool ID before the real `call_id` is known; preserve normal call_id and argument suffix semantics.
6. Run local tests, fmt, clippy, then build an isolated binary on 11449. Do not touch `/etc/cc-proxy/config.toml`, `/usr/local/bin/cc-proxy`, `11441`, or `/home/claude/.cc-connect/config.toml`.
7. Live verify direct upstream summary, isolated Anthropic non-stream/SSE thinking, tools and fresh-ID continuation; classify cache_read/cache_creation/miss/timeout separately. Do not claim cache preservation from hashes alone.

## Cache invariants
- `reasoning.summary` is response-control only; it is never inserted into the next Responses `input`.
- `static_prefix_hash` remains based on the old cache-relevant reasoning shape (`effort` only); add separate summary-mode telemetry.
- `history_prefix_hash` and `wire_input_hash` continue to hash actual input only.
- No `CODEMERMAFROST_RELOCATE` changes, no synthetic tail changes, no Chat fallback, no sidecar inference request.

## Deployment boundary
This takeover changes source/build artifacts and performs isolated 11449 validation only. Production deployment requires a separate explicit approval after live gates pass.
