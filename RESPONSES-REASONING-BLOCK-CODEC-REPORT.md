# Responses Reasoning Block Codec 验证报告

## 范围

- 工作树：`/root/projects/codewhale-proxy/source`
- 分支：`feat/gpt-responses-transport`
- 本次只在已有 dirty 工作树上增补 Responses codec 测试与实现；未 reset/clean，未修改 `/etc/cc-proxy/config.toml`、`/home/claude/.cc-connect/config.toml`、生产 11441，也未启动常驻代理。
- 已存在的工作树改动保持原样；本报告是本次新增交付物。

## 修改文件

- `src/responses/response.rs`
  - readable `reasoning.summary[].text` 转换为标准 Anthropic `thinking` block。
  - 明确不把 `encrypted_content` 当普通思考文本，也不伪造 encrypted reasoning signature；signature 保持空值。
  - 新增非流式回归测试。
- `src/responses/stream.rs`
  - `response.reasoning_summary_text.done` 在没有 delta 或仅有部分 delta 时只补齐未发送的 summary suffix，避免重复拼接。
  - 记录工具参数 delta，`response.function_call_arguments.done` 只发送未见 suffix。
  - 通过 call_id/item_id 映射抑制重复 `response.output_item.added`，工具 block stop 只发送一次。
  - 处理 Responses `error` 事件为 Anthropic `error` SSE，并终止后续事件。
  - 新增 reasoning、tool argument、重复 item、error 的回归测试。
- `RESPONSES-REASONING-BLOCK-CODEC-REPORT.md`
  - 本报告。

## TDD 证据

### RED

先加入缺失行为测试并运行，真实失败如下：

```text
reasoning_summary_done_emits_full_text_when_no_delta_arrived: FAILED
function_call_arguments_done_emits_only_unseen_suffix: FAILED
duplicate_function_call_item_added_does_not_start_a_second_tool_block: FAILED
responses_error_event_becomes_anthropic_error_and_stops_processing: FAILED
```

失败分别表现为 summary done 没有 `thinking_delta`、参数 done 没有 suffix、重复 item 产生额外事件、error event 没有 `api_error`。

### GREEN

补充最小状态字段和事件分支后，实际测试结果：

```text
cargo fmt --check                                      PASS
cargo test --locked responses::stream::tests           15 passed, 0 failed
cargo test --locked responses::response::tests          4 passed, 0 failed
```

先前已有基线运行结果（修改本轮之前）为 `86 passed, 0 failed`；本轮新增后相关 Responses 测试为 `19 passed, 0 failed`。

## 行为边界与证据

### 非流式 reasoning summary

响应 item 为：

```json
{"type":"reasoning","summary":[{"type":"summary_text","text":"short summary"}],"encrypted_content":"ciphertext"}
```

实际断言：输出只有一个 `thinking` block，`thinking == "short summary"`，`signature == ""`。仅含 `encrypted_content` 的 reasoning item 不产生普通 thinking 文本。

### Responses SSE reasoning

- delta 事件开启一个 Anthropic `thinking` block，并发送 `thinking_delta`。
- done 事件带完整 `text` 时按已发送 reasoning 前缀计算 suffix；若之前没有 delta，则发送完整 text；若 delta 已覆盖完整 text，则不重复发送。
- text/reasoning 切换时先发送当前 block 的 `content_block_stop`。

### 工具 block 与真实 ID

- `output_item.added(function_call)` 同时登记 `call_id` 与 item `id`，Anthropic `tool_use.id` 优先使用真实 `call_id`。
- 参数 delta 按对应 item/call ID 累积。
- arguments done 只补发送未见的参数后缀，因此不会把已经发送的 delta 再拼接一次。
- 重复 output item added 不再开启第二个 block。
- output item done 与 terminal finish 对同一工具 block 不重复发送 stop。

### unknown / failure / incomplete / terminal

- unknown Responses event 继续忽略，不改变现有状态。
- `response.incomplete` 保留 terminal 收尾，`max_output_tokens`/`max_tokens` 映射为 Anthropic `max_tokens`，其他 incomplete reason 保持 `end_turn` 边界。
- `response.failed`/`response.error` 转换为 Anthropic `error` SSE；`error` 事件同样转换并阻止后续事件。
- terminal `finish()` 幂等，重复调用不生成额外 block stop。

## 工具续接与请求边界

本轮确认并保持既有 request converter 的映射：

```text
function_call.call_id       -> tool_use.id
function_call_output.call_id -> tool_result.tool_use_id
```

`src/responses/request.rs` 仍将 `tool_use` 序列化为 `function_call`，`tool_result` 序列化为 `function_call_output`，参数使用 JSON 字符串。reasoning summary 不进入下一轮 Responses input；请求 converter 对 Anthropic `thinking`/`redacted_thinking`/unknown block 继续跳过。既有 `instructions`、input 顺序、tools 排序、prefix/cache hash 及 Chat 路由未在本轮新增逻辑中改变。

## 实际验证命令

```text
cargo test --locked                                      PASS（本轮之前基线，86 passed）
cargo fmt --check                                        PASS
cargo test --locked responses::stream::tests             PASS（15 passed）
cargo test --locked responses::response::tests           PASS（4 passed）
```

尚未在本轮运行的质量门：

```text
cargo test --locked（全量，修改后）                     PASS（91 passed, 0 failed）
cargo clippy --locked -- -D warnings                    PASS
```

没有执行真实 upstream A/B、Claude Code CLI 或生产 11441 验证；上游/CLI 边界沿用既有报告中的 CONDITIONAL/BLOCKED 结论，不能冒充本轮完成。

## 剩余限制

1. Responses summary 可读文本没有 Anthropic 原生签名；本实现明确不伪造 signature。下游若要求可验证签名，需要上游提供可验证的 Anthropic signature，而不是由 proxy 生成。
2. encrypted/redacted reasoning 仍不展示为普通思考文本，也不参与下一轮 input。
3. 本轮单元测试覆盖 codec 状态机与转换边界；未进行真实 upstream 长前缀 cache、tool continuation、Claude Code CLI 或生产服务验收。
4. 共享 dirty 工作树中其余已有变更没有被本轮重置或清理。
