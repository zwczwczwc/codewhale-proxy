# Responses 旁路全链路验收报告

- 验收时间：2026-08-05（本次运行以命令实际输出为准）
- 结果：**CONDITIONAL**
- 范围：仅开发分支 `feat/gpt-responses-transport`，临时监听 `127.0.0.1:11449`；未重启、修改或调用生产服务。
- 上游：`http://clawbot:11434`，DNS 解析为 `100.64.0.1`。本次运行时 `/v1/responses` 探测返回 HTTP 200。
- 凭证：仅通过受控运行时环境变量占位注入；本报告、日志和命令输出不包含 token 或 Authorization 值。

## 1. 范围和工作树核对

实际命令与结果摘要：

```text
git branch --show-current                 -> feat/gpt-responses-transport
git status --short                         -> 既有修改/未跟踪文件，未覆盖或回滚
ss -ltnp | grep ':11449|:11434'            -> 11449 未监听；11434 为 ollama(pid 3429)
getent hosts clawbot                       -> 100.64.0.1 clawbot.hermes.tailnet clawbot
cargo build --locked                       -> exit 0
cargo test --locked                        -> exit 0；69 passed, 0 failed
```

已读取基线文档：

- `CC-PROXY-RESPONSES-CONTEXT-RECOVERY.md`
- `GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md`
- `RESPONSE-E2E-AVAILABILITY-REPORT.md`

配置确认：`config.toml` 中仅 `gpt-5.6-luna` profile 设置 `wire_api = "responses"`；DeepSeek/GLM/Kimi profiles 未设置该字段，按默认仍为 Chat Completions。

## 2. 临时实例和清理

启动命令（凭证值未写入命令；这里使用运行时占位）：

```text
LISTEN_ADDR=127.0.0.1:11449
ESWITCH_URL=http://clawbot:11434
MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml
RUST_LOG=info
/root/projects/codewhale-proxy/source/target/debug/cc-proxy
```

实际结果：

```text
启动进程：cc-proxy 子进程 PID 820531（外层 shell PID 820515）
日志：Loaded 5 model profiles, 4 providers
日志：Listening on 127.0.0.1:11449
日志：eswitch health check: OK
日志：Server ready on 127.0.0.1:11449
```

实例使用独立临时端口，未触碰 `11441` 生产 cc-proxy，也未改动 `/etc`、systemd 或生产配置。验收完成后执行了终止临时进程并确认 `11449` 不再监听（见末尾清理记录）。

## 3. 实际 E2E 结果

### 3.1 Claude Code 对外 Anthropic `/v1/messages` 普通文本

未直接运行 Claude Code CLI：本机 CLI 会自动读取 `/root/.claude` 生产/默认配置入口，无法在不读取或覆盖该配置的前提下证明它使用隔离实例。因此使用等价、显式指向临时 HTTP 端口的 Anthropic API 请求验证协议链路。

```text
POST http://127.0.0.1:11449/v1/messages
HTTP 200
model: gpt-5.6-luna
stop_reason: end_turn
content types: [text]
text: "bypass ordinary text ok"
usage keys: input_tokens, output_tokens, cache_read_input_tokens, cache_creation_input_tokens
```

结论：**PASS（协议级普通文本）**。

### 3.2 xhigh

发送 Anthropic `thinking: {"type":"adaptive"}` 请求。临时代理日志确认构建 Responses 请求，例如：

```text
Responses request built prefix_fingerprint=c8d1a50d18bd318f model=gpt-5.6-luna
HTTP 200
stop_reason: end_turn
text: "xhigh path ok"
```

实现配置/基线要求将 GPT Responses reasoning 映射为 `reasoning.effort=xhigh`；本次响应成功，但代理日志未打印请求体，故不在报告中泄露请求内容。结论：**PASS（端到端响应成功；请求字段由代码路径映射）**。

### 3.3 Function tool 与两轮 continuation

首轮显式 tool 请求实际结果：

```text
HTTP 200
stop_reason: tool_use
content types: [tool_use]
tool name: lookup_weather
usage keys: input_tokens, output_tokens, cache_read_input_tokens, cache_creation_input_tokens
```

SSE tool 请求实际结果：

```text
HTTP 200
content-type: text/event-stream
events: 11
包含 content_block_start(type=tool_use)
包含多个 content_block_delta(delta.type=input_json_delta)
包含 message_delta(stop_reason=tool_use)
包含 message_stop
```

但是提交 `tool_result` 的 continuation 在本次运行中未稳定完成：一次客户端超时（Python `TimeoutError`），另一次返回 HTTP 502；按要求不将其记为成功。因此两轮 continuation：**BLOCKED/未通过**。该行为/协议缺陷留给 reviewer 判断，本任务不修改源代码。

### 3.4 SSE 文本

```text
POST /v1/messages, stream=true
HTTP 200
content-type: text/event-stream
bytes: 857
event_count: 7
包含 content_block_delta/text_delta: "stream", " ok"
包含 message_delta(stop_reason=end_turn)
```

结论：**PASS（SSE 文本）**。

### 3.5 DeepSeek/GLM/Kimi Chat 回归

使用临时旁路端口分别发送普通文本请求：

```text
deepseek-v4-flash -> HTTP 200, stop=end_turn, model=deepseek-v4-flash
glm-5.2           -> HTTP 200, stop=end_turn, model=glm-5.2
kimi-k3           -> HTTP 200, stop=max_tokens, model=kimi-k3
```

三者均由非 Responses profile 路径处理；Kimi 的 `max_tokens` 是响应行为，不是 HTTP/路由失败。结论：**CONDITIONAL PASS（路由与 HTTP 成功；Kimi 输出截断）**。

## 4. Cache 观测

使用同一稳定 system 前缀连续四次请求，实际 usage 如下：

| 请求 | input_tokens | output_tokens | cache_read_input_tokens | cache_creation_input_tokens |
|---:|---:|---:|---:|---:|
| 1 | 259 | 25 | 0 | 0 |
| 2 | 259 | 6 | 0 | 0 |
| 3 | 未完成（客户端 TimeoutError） | — | — | — |
| 4 | 259 | 26 | 0 | 0 |

代理输出的原始字段名为 `cache_read_input_tokens` 与 `cache_creation_input_tokens`；未观察到 `cached_tokens` 或 `cache_write_tokens`，因此没有改名或伪装字段。

第 3/4 次命中率：**未验证**。可完成的第 4 次实际命中率为 `0 / 259 = 0%`；第 3 次没有响应，不能算命中或未命中。短前缀长度仅 259 input tokens，也不足以复现基线文档中约 4073 token 的 cache 试验，故不能据此否定上游 cache 能力。

## 5. 证据与安全边界

- 正确上游证据：`getent hosts clawbot` 返回 `100.64.0.1`；直连 `http://clawbot:11434/v1/responses` 探测 HTTP 200。
- 生产 `127.0.0.1:11434` 未被用于 Responses 验收；该地址进程归属为 Ollama。
- 临时实例只监听 `127.0.0.1:11449`，不接收外部网络流量。
- 没有执行生产部署、systemd 操作、服务重启或配置写入。
- 没有在报告、日志、评论或命令输出中打印 token、Authorization 或 secret 值。
- 未使用 `previous_response_id`，未启用 encrypted reasoning replay，未设置 Responses 失败后的 Chat fallback。

## 6. 清理和最终状态

```text
临时实例：PID 820531（外层启动 shell 820515）
清理动作：发送终止信号，随后确认进程退出
最终检查：11449 无监听；生产监听/配置未修改
```

原始工作树中的既有代码修改和三个输入文档均保持原样；本次只新增本报告文件。

## 7. 本次旁路复验（t_f964c47f）

### 7.1 临时实例

```text
启动：env LISTEN_ADDR=127.0.0.1:11449 ESWITCH_URL=http://clawbot:11434 \
      MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml \
      RUST_LOG=info target/debug/cc-proxy
启动 PID：850181（Hermes process session proc_20eaaebc3be6）
日志：Loaded 5 model profiles, 4 providers；eswitch health check: OK；
      Server ready on 127.0.0.1:11449
清理：SIGTERM PID 850181；最终 ss 未发现 11449 监听；清理请求命令退出码 0
```

### 7.2 Responses function-call continuation（真实 HTTP）

使用临时端口和运行时占位凭证，未打印 Authorization/token。请求均为 Anthropic `/v1/messages`，没有 Chat fallback：

```text
ROUND1 function_call: HTTP 200, application/json, stop_reason=tool_use,
  content_types=[tool_use], tool=lookup_weather, tool id present=true
ROUND1 follow-up: 客户端 120.06s 超时，实际输出为 timed out（不计为通过）
ROUND2 function_call: HTTP 200, stop_reason=tool_use, tool id present=true
ROUND2 follow-up: HTTP 200, application/json, stop_reason=end_turn,
  content_types=[text]
ROUND2 follow-up: HTTP 200, application/json, stop_reason=end_turn,
  content_types=[text]（第二个 tool continuation 完成）
```

结论：**CONDITIONAL**。至少一条真实两轮链路通过，但重复测试首轮 follow-up 发生 120 秒超时，故不能声称 continuation 稳定通过；超时保留为失败证据。

### 7.3 长前缀 cache（真实 usage）

固定 `instructions`（约 1130 input tokens）与固定 `lookup_weather` tool，连续四次请求：

```text
CACHE request1: HTTP 200, usage input=1133, output=8,
  cache_creation_input_tokens=1130, cache_read_input_tokens=0
CACHE request2: 客户端 120.10s 超时，输出 timed out（usage 不可得）
CACHE request3: HTTP 200, usage input=1133, output=8,
  cache_creation_input_tokens=1130, cache_read_input_tokens=0
CACHE request4: HTTP 200, usage input=1133, output=8,
  cache_creation_input_tokens=1130, cache_read_input_tokens=0
```

第 3/4 次可观测 cache 命中率为 `0 / 1130 = 0%`；第 2 次无响应，不计命中或未命中。四次中 3 次可解析响应，HTTP 成功率 `3/4=75%`。usage 使用代理真实字段名，没有改写成 `cached_tokens`。

### 7.4 Claude Code CLI

`claude --version` 实际为 `2.1.209`。尝试使用临时 `HOME`/`CLAUDE_CONFIG_DIR` 和临时端口执行 CLI 时被当前执行环境的安全审批拦截，未执行成功；因此普通、xhigh、tool/continuation/streaming 的 Claude Code CLI 验收均为 **BLOCKED**，没有把协议级 API 结果冒充 CLI 结果，也未读取或修改 `/root/.claude`。

## 最终判定

**CONDITIONAL**：Responses 普通文本、xhigh 响应、非流式 tool call、SSE 文本和 SSE function-call 事件，以及 DeepSeek/GLM/Kimi Chat 路由均有真实 HTTP 证据；但 tool continuation 未稳定成功，长稳定前缀第 3/4 次 cache 命中率未验证，且 Claude Code CLI 因会自动读取默认/生产配置而未直接运行。不得据此批准生产部署。

## 8. 本次重试验收（t_5d8be2e0）

- 工作树前置检查：`src/responses/request.rs` 已包含 assistant `output_text` 单测；未修改源代码。
- `cargo test --locked`：**77 passed, 0 failed**。
- 直接上游健康探测（认证占位头）：`http://clawbot:11434/v1/responses` **HTTP 200**。
- 临时旁路：启动 `127.0.0.1:11449 -> http://clawbot:11434`，日志显示 health check OK / Server ready；仅使用 `Authorization: Bearer ***`，未输出 secret。
- 普通多轮/文本：HTTP 200，`stop=end_turn`。
- SSE：HTTP 200，`text/event-stream`，64 events，含 `text_delta` 与 `message_stop`。
- 真实 tool continuation：首轮 HTTP 200、`stop=tool_use`、存在真实 call id；带该 call id 的 continuation HTTP 200、`stop=end_turn`、返回文本。**PASS（本次单次链路）**。
- 长前缀 cache 三次：第 1 次 HTTP 200，第 2 次客户端 40s `TimeoutError`，第 3 次 HTTP 200；三次均能看到真实 usage 字段，但未取得三次有效 200，因此 cache 三次验收 **未通过/未完成**，不能伪造命中率。
- 清理：临时旁路已 SIGTERM 并退出；最终 `11449` 无监听。生产 `11441` 未触碰，最终 health **HTTP 200**，仍由原 `cc-proxy` 进程监听。

本次结论：**CONDITIONAL**。上游 cache 长前缀第二次请求超时是本次旁路的真实失败原因；不将其归因于 `output_text` 修复，也不批准生产部署。
