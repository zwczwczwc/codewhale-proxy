# cc-proxy 支持 gpt-5.6 Responses API 改造方案

> **执行说明**：下一会话推进实际开发时，必须先阅读本文，再按“实施阶段”和“TDD 垂直切片”执行。本文是设计与验收基线，不是已经完成的代码变更。
>
> **范围**：改造现有 Rust `cc-proxy`，不替换它、不新增独立代理服务。

## 1. 目标

在不增加新的常驻代理、不恢复 `proxy.py`、不恢复 `cc-switch-cli`、不破坏现有 Chat Completions 路径的前提下，使 Claude Code 能够通过现有 `cc-proxy` 使用：

- `gpt-5.6` 系列模型；
- `reasoning.effort=max`；
- Function tools；
- tool result 多轮续接；
- 流式文本和流式工具调用；
- 可观测且尽量稳定的 Responses API prefix cache hit。

目标链路：

```text
Claude Code
  │ Anthropic Messages /v1/messages
  ▼
cc-proxy（现有 Rust 服务）
  ├─ deepseek / glm / kimi
  │    └─ /v1/chat/completions（现有路径，保持不变）
  │
  └─ gpt-5.6 系列
       └─ /v1/responses（新增上游路径）
  ▼
eswitch :11434
  ▼
llmapi.efunds.com.cn
```

Claude Code 对外仍然调用 `cc-proxy` 的 Anthropic `/v1/messages`；只有 `cc-proxy → eswitch` 的内部上游 wire API 对 gpt-5.6 模型切换为 `/v1/responses`。

## 2. 明确不做的事情

本次改造明确不做：

1. 不替换现有 `cc-proxy`；
2. 不把 `mxyhi/token_proxy` 整体部署成新的长期运行服务；
3. 不新增 `proxy.py`、cc-switch、nginx 或其他中间代理；
4. 不重写整个 cc-proxy 为新的统一网关；
5. 不改变 DeepSeek、GLM、Kimi 的现有 Chat Completions 传输路径；
6. 不让 Responses 请求失败后隐式降级到 Chat Completions；
7. 第一阶段不默认使用 `previous_response_id`；
8. 第一阶段不伪造或强制 replay gpt Responses 的 encrypted reasoning；
9. 第一阶段不新增 cc-proxy 对外的 OpenAI `/v1/responses` 入站端点；
10. 不删除或绕过现有 permafrost/cache 前缀稳定化逻辑。

## 3. 当前实际背景

### 3.1 当前部署

| 节点 | 服务 | 端口 | 上游 |
|---|---|---:|---|
| GPU 节点 | `cc-proxy` Rust | `11441` | `http://clawbot:11434` |
| clawbot | `cc-proxy` Rust | `11435` | `http://127.0.0.1:11434` |
| clawbot | `eswitch` 官方 Go 服务 | `11434` | `http://llmapi.efunds.com.cn/v1/yfd/openclaw` |

当前 `11434` 已恢复为官方 Go 编写的 `eswitch`。此前自建的 Python `proxy.py` 已停用，`cc-switch-cli` 也已停用。

### 3.2 当前代码仓库

源码目录：

```text
/root/projects/codewhale-proxy/source/
```

远程仓库：

```text
https://github.com/zwczwczwc/cc-proxy
```

当前 HEAD（设计文档创建时现场确认）：

```text
12dee7e Merge pull request #1 from zwczwczwc/feat/default-effort-xhigh
```

当前仓库状态：

```text
master 分支；工作树干净；未发现 AGENTS.md 或 CLAUDE.md
```

关键文件：

```text
src/client.rs
src/config.rs
src/routes/messages.rs
src/anthropic/converter.rs
src/anthropic/types.rs
src/openai/types.rs
src/openai/converter.rs
src/sse/stream.rs
src/reasoning/build_messages.rs
src/reasoning/relocate.rs
src/reasoning/prefix.rs
config.toml
```

当前请求路径：

```text
src/routes/messages.rs
  → anthropic::converter::convert_request()
  → ChatCompletionRequest
  → DeepSeekClient::chat_completion[_stream]()
  → eswitch /v1/chat/completions
```

当前 `src/client.rs` 只有 Chat 相关方法：

```rust
chat_completion()
chat_completion_stream()
health_check()
```

当前没有 Responses client、Responses request converter、Responses response converter 或 Responses SSE state machine。

### 3.3 当前模型映射

当前配置已经包含：

```toml
"claude-sonnet-4-6" = "gpt-5.6-luna"
```

并已有 gpt profile：

```toml
[[model_profiles]]
name = "gpt-5.6-luna"
provider = "gpt"
reasoning_enabled = true
reasoning_replay = false
toolcall_requires_reasoning = false
aliases = ["gpt-5.6-luna-2026-07-09"]
```

当前 profile 缺少显式的 wire API 字段，因此路由仍然只能走现有 Chat 路径。

## 4. 已确认的上游能力

以下结论来自对 clawbot 本机 `eswitch` 的实际请求验证，不是依据模型名称或 README 推断。

### 4.1 gpt-5.6-luna 能力矩阵

| 请求 | 实际结果 |
|---|---|
| `/v1/chat/completions` + tools | `200`，返回 `tool_calls` |
| `/v1/chat/completions` + `reasoning_effort=xhigh` | `200` |
| `/v1/chat/completions` + tools + `reasoning_effort=xhigh` | `400` |
| `/v1/responses` + tools | `200`，返回 `function_call` |
| `/v1/responses` + tools + `reasoning.effort=xhigh` | `200`，返回 `reasoning/function_call` |
| `/v1/responses` 流式 + tools | 正常返回 Responses 事件流 |
| `function_call_output` 续接 | `200`，返回最终 message |

Chat 组合的上游错误原文：

```text
Function tools with reasoning_effort are not supported for gpt-5.6-luna
in /v1/chat/completions. Please use /v1/responses instead.
```

因此，以下组合必须走 Responses：

```text
gpt-5.6-luna + tools + xhigh
```

不能通过“继续走 Chat”或“Responses 失败后退回 Chat”解决。

### 4.2 已确认的 Responses 流式事件

实际抓到过：

```text
response.created
response.in_progress
response.output_item.added
response.function_call_arguments.delta
response.function_call_arguments.done
response.output_item.done
response.output_text.delta
response.output_text.done
response.content_part.added
response.content_part.done
response.completed
```

实现还应兼容：

```text
response.reasoning_summary_text.delta
response.reasoning_summary_text.done
response.reasoning.delta
response.reasoning.done
response.incomplete
response.failed
error
```

### 4.3 已确认的 tool continuation

第一轮 Responses 返回：

```json
{
  "type": "function_call",
  "call_id": "call_xxx",
  "name": "get_weather",
  "arguments": "{\"city\":\"北京\"}"
}
```

下一轮提交：

```json
{
  "type": "function_call_output",
  "call_id": "call_xxx",
  "output": "北京晴，25摄氏度。"
}
```

上游可以返回最终 message。

### 4.4 已确认的 Responses cache 行为

使用约 4073 token 的稳定前缀连续请求：

```text
第 1 次：input_tokens=4073，cache_write_tokens=4070，cached_tokens=0
第 2 次：input_tokens=4073，cache_write_tokens=4070，cached_tokens=0
第 3 次：input_tokens=4073，cache_write_tokens=0，cached_tokens=4070
第 4 次：input_tokens=4073，cache_write_tokens=0，cached_tokens=4070
```

本次隔离测试中预热后的命中率约为：

```text
4070 / 4073 ≈ 99.9%
```

注意：短 prompt、首次请求或预热阶段的 `cached_tokens=0` 不能证明 Responses 没有缓存。

## 5. GitHub 参考项目与采用边界

### 5.1 `mxyhi/token_proxy`：Rust 协议实现参考

仓库：

<https://github.com/mxyhi/token_proxy>

本次调研使用的最新 commit：

```text
6bed3d1ebbbb44c06833d37b34b2ebe49cc8d8a2
```

许可证：

```text
Apache-2.0
```

关键文件：

```text
crates/token_proxy_runtime/src/proxy/anthropic_compat/request.rs
crates/token_proxy_runtime/src/proxy/anthropic_compat/response.rs
crates/token_proxy_runtime/src/proxy/anthropic_compat/tests.rs
crates/token_proxy_runtime/src/proxy/server/dispatch.rs
```

可参考：

- Anthropic Messages → Responses request；
- `system`、messages、tools、tool_choice；
- `tool_use` → `function_call`；
- `tool_result` → `function_call_output`；
- reasoning；
- Responses → Anthropic tool_use；
- stop reason；
- Responses provider 路由；
- Rust 测试组织。

采用边界：

- 可以参考其结构、字段映射和测试设计；
- 如直接移植代码，必须保留 Apache-2.0 许可证和版权义务；
- 不整体替换当前 cc-proxy；
- 不直接照搬其可能与 EFund/eswitch 不一致的 system、header、usage 逻辑。

### 5.2 `tangsipeng/openai-responses-anthropic-proxy`：tool continuation 与 SSE 参考

仓库：

<https://github.com/tangsipeng/openai-responses-anthropic-proxy>

本次调研使用的最新 commit：

```text
cdba6dadd8625f27910cded16b22f6bd797d1aff
```

关键文件：

```text
src/translate.ts
src/server.ts
src/state.ts
src/server.test.ts
```

可参考：

- Responses function call 流式状态机；
- `previous_response_id` 续接；
- tool continuation fallback；
- reasoning/refusal/incomplete；
- cache usage 解析；
- 测试事件序列。

本次未确认其仓库许可证，因此不直接复制代码；仅作为协议和测试参考。

### 5.3 `Lokesh-Chimakurthi/rosetta-llm`：codec 和事件模型参考

仓库：

<https://github.com/Lokesh-Chimakurthi/rosetta-llm>

本次调研使用的最新 commit：

```text
0c86c36ceeb414416f8b067a9f2b312f1fb85eab
```

关键文件：

```text
src/rosetta/codecs/openai_responses.py
src/rosetta/stream_codecs/openai_responses.py
src/rosetta/pipeline.py
```

它采用 Canonical IR 和独立 stream codec。其定向 round-trip 测试结果为：

```text
9 passed
```

完整测试为：

```text
17 passed，1 个 Chat passthrough 测试失败
```

采用边界：

- 参考其独立 codec、事件模型和测试边界；
- 第一阶段不对当前 cc-proxy 做全量 IR 重构，以避免扩大 DeepSeek 现有路径的回归面。

### 5.4 `musistudio/claude-code-router`：缓存和 provider 参数风险参考

仓库：

<https://github.com/musistudio/claude-code-router>

本次调研使用的 commit：

```text
bc8a8e62051793d71a0378643b0bb45affc05873
```

许可证：MIT。

重点 Issue：

- <https://github.com/musistudio/claude-code-router/issues/1372>
- <https://github.com/musistudio/claude-code-router/issues/1515>

Issue #1372 记录了 Claude Code 动态 `x-anthropic-billing-header/cch` 原样进入 Responses system/instructions 后导致 cache prefix 失效的问题。

Issue #1515 记录了 Responses provider 无条件转发不支持的 `thinking` 字段导致上游 400。

这些问题直接说明：

1. billing header 必须在稳定前缀之外处理；
2. `thinking`、`reasoning_effort` 等字段必须按 wire API 和 provider 能力过滤；
3. 不能只根据模型名字推断所有参数都可以发送。

## 6. 总体架构设计

### 6.1 保留 Chat 路径

以下模型保持当前路径：

```text
deepseek-v4-pro
deepseek-v4-flash
glm-5.2
kimi-k3
```

继续使用：

```text
Anthropic Messages
  → Chat Completions converter
  → eswitch /v1/chat/completions
```

不得因为新增 Responses 路径而改变其请求格式、reasoning replay、tool result 修复和现有 SSE 行为。

### 6.2 新增 Responses 路径

仅 profile 声明：

```toml
wire_api = "responses"
```

的模型进入：

```text
Anthropic Messages
  → Responses converter
  → eswitch /v1/responses
```

第一阶段只启用：

```text
gpt-5.6-luna
```

### 6.3 不使用模型名前缀自动判断

禁止依赖：

```rust
model.starts_with("gpt-5.6")
```

使用显式 `ModelProfile.wire_api`，因为不同模型变体的 Responses 能力必须逐个实测确认。

## 7. 具体代码改造

### 7.1 增加模型级 `WireApi`

修改：

```text
src/config.rs
```

新增：

```rust
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WireApi {
    #[serde(rename = "chat_completions")]
    ChatCompletions,

    #[serde(rename = "responses")]
    Responses,
}

impl Default for WireApi {
    fn default() -> Self {
        Self::ChatCompletions
    }
}
```

扩展 `ModelProfile`：

```rust
#[serde(default)]
pub wire_api: WireApi,
```

增加：

```rust
pub fn wire_api_for_model(&self, model: &str) -> WireApi
```

要求：

- 未声明 `wire_api` 的旧 profile 默认 Chat；
- canonical name 与 alias 都能得到相同 profile；
- 非法值在配置校验阶段明确失败；
- 不修改现有 profile 的默认行为。

### 7.2 配置 gpt-5.6-luna

修改 `config.toml`：

```toml
[[model_profiles]]
name = "gpt-5.6-luna"
provider = "gpt"
wire_api = "responses"
reasoning_enabled = true
reasoning_replay = false
toolcall_requires_reasoning = false
aliases = ["gpt-5.6-luna-2026-07-09"]
```

其他模型不添加该字段。

### 7.3 新增 Responses 模块

新增：

```text
src/responses/mod.rs
src/responses/types.rs
src/responses/request.rs
src/responses/response.rs
```

职责：

```text
types.rs
  Responses request/response 数据类型

request.rs
  Anthropic Messages → Responses request

response.rs
  Responses response → Anthropic response
```

对 EFund 可能扩展的 Responses item，优先保留 `serde_json::Value`，不要因为未知字段直接让整个请求解析失败。

### 7.4 扩展 `src/client.rs`

新增：

```rust
pub async fn responses_completion(
    &self,
    request: &ResponsesRequest,
) -> anyhow::Result<Value>
```

以及：

```rust
pub async fn responses_completion_stream(
    &self,
    request: &ResponsesRequest,
) -> anyhow::Result<impl Stream<Item = reqwest::Result<bytes::Bytes>>>
```

请求 URL：

```text
{eswitch_url}/v1/responses
```

复用当前 client 的：

- Authorization header；
- Content-Type；
- connect/read timeout；
- connection pool；
- 错误脱敏。

Responses 4xx 不得自动改走 Chat。

### 7.5 system → instructions

复用当前已有的前处理：

```text
stabilize_metadata()
→ migrate_volatile_system_blocks()
→ 合并稳定 system blocks
→ Responses instructions
```

输出：

```json
{
  "instructions": "稳定系统提示"
}
```

要求：

- Claude Code 的动态 `cch` 不能进入稳定 instructions；
- system block 顺序稳定；
- 固定换行规则；
- 不加入 request ID、时间、随机值；
- 动态环境内容继续迁移到最后 user turn；
- 相同稳定前缀产生相同字节序列。

### 7.6 user/assistant 历史

user：

```json
{
  "role": "user",
  "content": [
    {
      "type": "input_text",
      "text": "用户输入"
    }
  ]
}
```

assistant 文本：

```json
{
  "type": "message",
  "role": "assistant",
  "status": "completed",
  "content": [
    {
      "type": "output_text",
      "text": "助手回复"
    }
  ]
}
```

图片沿用现有 Anthropic image 解析，转换为 Responses `input_image`。

### 7.7 tool_use → function_call

Anthropic：

```json
{
  "type": "tool_use",
  "id": "toolu_123",
  "name": "get_weather",
  "input": {
    "city": "北京"
  }
}
```

Responses：

```json
{
  "type": "function_call",
  "call_id": "toolu_123",
  "name": "get_weather",
  "arguments": "{\"city\":\"北京\"}"
}
```

要求：

- `call_id` 稳定；
- 后续 tool result 使用同一 ID；
- arguments 采用稳定紧凑 JSON；
- 参数序列化失败必须返回错误，不得静默变成空对象；
- 多个工具调用按原始顺序保留。

### 7.8 tool_result → function_call_output

Anthropic：

```json
{
  "type": "tool_result",
  "tool_use_id": "toolu_123",
  "content": "北京晴，25℃"
}
```

Responses：

```json
{
  "type": "function_call_output",
  "call_id": "toolu_123",
  "output": "北京晴，25℃"
}
```

### 7.9 工具排序和稳定序列化

为保护 prefix cache：

1. 工具按 `name` 升序；
2. tool schema JSON key 稳定排序；
3. 使用稳定、无随机字段的序列化；
4. 不加入时间、request ID、随机 tool ID；
5. 保持工具 description 和 parameters 语义不变；
6. `tool_choice` 和 `parallel_tool_calls` 单独转换。

### 7.10 reasoning

gpt Responses 只发送：

```json
{
  "reasoning": {
    "effort": "xhigh"
  }
}
```

禁止发送：

```json
"reasoning_effort": "xhigh"
```

禁止发送：

```json
"thinking": {
  "type": "enabled"
}
```

第一阶段策略：

```toml
reasoning_replay = false
```

只有 Responses 返回可安全复用的 reasoning encrypted content，并完成 issuer/会话验证后，才另行设计 replay。

### 7.11 max tokens

映射：

```text
Anthropic max_tokens
→ Responses max_output_tokens
```

上游实测最小值为 16。小于 16 时，建议 cc-proxy 本地返回明确的 `invalid_request_error`，不要静默扩大请求。

### 7.12 非流式响应

Responses `output` 中：

```text
message
function_call
reasoning
```

映射：

```text
message/output_text → Anthropic text
function_call        → Anthropic tool_use
reasoning summary    → Anthropic thinking（仅有可见 summary 时）
```

Arguments 解析失败必须返回转换错误。

stop reason：

```text
function_call                 → tool_use
completed + 普通 message     → end_turn
incomplete/max_output_tokens  → max_tokens
failed                        → error
```

### 7.13 usage/cache 映射

Responses：

```json
{
  "input_tokens": 4073,
  "output_tokens": 5,
  "input_tokens_details": {
    "cached_tokens": 4070,
    "cache_write_tokens": 0
  }
}
```

Anthropic：

```json
{
  "input_tokens": 4073,
  "output_tokens": 5,
  "cache_read_input_tokens": 4070,
  "cache_creation_input_tokens": 0
}
```

要求：

- `input_tokens` 不减 cached tokens；
- 不混淆 cache read、cache write、cache miss；
- 同时支持 Responses 的 `input_tokens_details` 和兼容字段；
- Responses cache 指标写入独立日志。

### 7.14 新增 Responses SSE 状态机

新增：

```text
src/sse/responses.rs
```

不修改现有：

```text
src/sse/stream.rs
```

需要处理：

```text
response.created
response.in_progress
response.output_item.added
response.output_text.delta
response.output_text.done
response.reasoning_summary_text.delta
response.reasoning_summary_text.done
response.function_call_arguments.delta
response.function_call_arguments.done
response.output_item.done
response.completed
response.incomplete
response.failed
error
```

建议状态：

```rust
struct FunctionCallState {
    output_index: u32,
    call_id: String,
    name: String,
    arguments: String,
    anthropic_content_index: Option<u32>,
}
```

工具事件行为：

1. `output_item.added(function_call)`：发送 `content_block_start(tool_use)`；
2. `function_call_arguments.delta`：发送 `input_json_delta`；
3. `function_call_arguments.done`：标记参数完成；
4. `output_item.done`：发送 `content_block_stop`；
5. `response.completed`：发送 `message_delta`、`message_stop`。

必须保证：

- `content_block_stop` 不重复；
- `response.completed` 不重复发送已经发送的 delta；
- 客户端断开后关闭上游 stream；
- 已产生首个客户端可见输出后不重试同一流；
- 上游异常可以转换为 Anthropic error event。

## 8. 路由改造

修改：

```text
src/routes/messages.rs
```

流程：

```rust
let upstream_model = map_model_to_upstream(&req.model, &config);
let wire_api = config.wire_api_for_model(&upstream_model);

match wire_api {
    WireApi::ChatCompletions => {
        // 现有代码路径，保持不变
    }
    WireApi::Responses => {
        // 新增 Responses 路径
    }
}
```

必须基于 `upstream_model` 查询：

```text
claude-sonnet-4-6
  → gpt-5.6-luna
  → responses
```

而：

```text
claude-sonnet-4-5
  → deepseek-v4-pro
  → chat_completions
```

gpt Responses 请求失败时不得隐式降级到 Chat。

## 9. Cache hit 保护与观测

### 9.1 必须保留的逻辑

Responses 路径复用：

```text
stabilize_metadata()
migrate_volatile_system_blocks()
工具排序
```

不复制一套不同版本的 permafrost 逻辑。

### 9.2 Responses prefix fingerprint

新增独立函数：

```rust
compute_responses_prefix_fingerprint(
    instructions: &str,
    tools: &[ResponsesTool],
) -> String
```

hash 输入：

```text
稳定序列化后的 instructions
+ 稳定序列化后的 Responses tools
```

不能只 hash 工具名，因为 schema 变化也会改变上游缓存前缀。

日志字段建议：

```text
wire_api=responses
model=gpt-5.6-luna
prefix_fingerprint=...
tool_count=...
reasoning_effort=xhigh
```

响应完成后记录：

```text
cache_hit=...
cache_write=...
cache_miss=...
prompt_tokens=...
hit_rate=...
```

### 9.3 不默认使用 previous_response_id

第一阶段默认完整历史重放：

```text
assistant tool_use
→ function_call

user tool_result
→ function_call_output
```

理由：

- cc-proxy 当前是无状态服务；
- GPU 和 clawbot 不共享会话状态；
- 进程重启不应破坏会话；
- 完整历史更容易审计和验证 cache hit。

后续可以增加：

```toml
responses_use_previous_response_id = false
```

只有完整历史路径稳定后再评估启用，并且必须实现 `previous_response_id` 失败时的完整历史 fallback。

### 9.4 Cache 验收阈值

使用固定、超过 1024 token 的 instructions 和固定 tools，连续请求至少四次：

```text
第 1 次允许 cache_write_tokens > 0
第 2 次允许仍处于预热
第 3/4 次 cached_tokens 应明显大于 0
```

验收目标：

```text
cached_tokens / input_tokens >= 90%
```

如果不达标，按顺序检查：

1. instructions 是否每轮变化；
2. `cch` 是否进入稳定前缀；
3. tools 顺序是否变化；
4. schema 序列化是否变化；
5. 是否有其他模型/会话造成上游缓存淘汰。

## 10. 实施阶段与 TDD 任务

每个产生代码的任务都必须遵循：

```text
RED：先写失败测试并确认失败原因正确
GREEN：写最小实现使测试通过
REFACTOR：保持测试通过后再整理代码
```

### 阶段 0：分支、现场和基线

文件/命令：

```bash
cd /root/projects/codewhale-proxy/source
git status --short
git remote -v
git fetch origin
git log origin/master --oneline -3
git checkout -b feat/gpt-responses-transport
cargo fmt --check
cargo test
```

验收：

- 工作树基线明确；
- 远程仓库为 `zwczwczwc/cc-proxy`；
- 现有测试基线记录；
- 不修改生产服务。

### 阶段 1：WireApi 配置

修改：

```text
src/config.rs
config.toml
```

测试：

- 默认 profile 为 Chat；
- gpt profile 为 Responses；
- alias 查询一致；
- 非法值失败。

### 阶段 2：共享 Anthropic 预处理

只在确有重复时抽取共享函数：

```text
模型映射
system 稳定化
volatile block relocation
thinking 状态解析
```

先补 Chat 回归测试，再抽取，确保现有 Chat payload 不变。

### 阶段 3：Responses request converter

新增：

```text
src/responses/types.rs
src/responses/request.rs
```

测试覆盖：

- system → instructions；
- user/assistant；
- tool_use；
- tool_result；
- tool_choice；
- parallel_tool_calls；
- tools 排序；
- schema 稳定序列化；
- xhigh；
- max_output_tokens；
- 不发送 `thinking` 和顶层 `reasoning_effort`。

### 阶段 4：Responses client

修改：

```text
src/client.rs
```

测试：

- 请求路径 `/v1/responses`；
- headers 正确；
- 2xx 解析；
- 4xx 错误传播；
- 不发生 Chat fallback；
- 未开始输出前可以按现有策略处理连接失败。

### 阶段 5：非流式 response converter

新增：

```text
src/responses/response.rs
```

测试：

- message；
- function_call；
- reasoning summary；
- incomplete；
- failed；
- stop reason；
- cache usage；
- malformed arguments。

### 阶段 6：Responses SSE

新增：

```text
src/sse/responses.rs
```

测试：

- text stream；
- reasoning stream；
- function-call stream；
- 参数分片；
- 重复 output_item.done；
- incomplete；
- failed；
- 无 `[DONE]`；
- 客户端断开。

### 阶段 7：模型路由

修改：

```text
src/routes/messages.rs
```

测试：

```text
gpt-5.6-luna       → /v1/responses
deepseek-v4-pro    → /v1/chat/completions
deepseek-v4-flash  → /v1/chat/completions
glm-5.2            → /v1/chat/completions
kimi-k3            → /v1/chat/completions
```

### 阶段 8：Responses 缓存观测

新增：

```text
Responses prefix fingerprint
Responses cache_hit/cache_write/cache_miss 日志
```

测试：

- 相同 instructions/tools 指纹稳定；
- 动态 user 内容不破坏稳定 prefix；
- Responses usage 的 cache 字段被读取；
- Chat 现有 KV cache 日志不改变。

### 阶段 9：真实 eswitch 集成

先使用旁路端口，不直接覆盖生产：

```text
LISTEN_ADDR=127.0.0.1:11449
ESWITCH_URL=http://100.64.0.1:11434
```

测试：

1. gpt-5.6-luna 非流式文本；
2. gpt-5.6-luna + xhigh；
3. gpt-5.6-luna + tools + xhigh；
4. tool result 续接；
5. 流式文本；
6. 流式 tool call；
7. 多轮请求；
8. 长稳定 instructions cache；
9. cached_tokens usage；
10. eswitch 日志确认 `/v1/responses`。

### 阶段 10：Claude Code E2E

临时让 Claude Code 使用：

```text
claude-sonnet-4-6 → gpt-5.6-luna
```

验证：

- 普通文本；
- terminal/file tool；
- 工具参数正确；
- tool result 后最终回答；
- 连续多轮；
- 流式输出；
- xhigh；
- 无 Chat 400；
- cc-proxy 日志为 `wire_api=responses`；
- eswitch 日志为 `POST /v1/responses`、HTTP 200。

## 11. 测试命令和部署计划

### 11.1 本地质量门

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

### 11.2 生产部署前现场确认

GPU 节点：

```text
/etc/cc-proxy/config.toml
/usr/local/bin/cc-proxy
监听 11441
```

clawbot：

```bash
systemctl cat cc-proxy
```

确认：

- 实际二进制路径；
- `MODEL_CONFIG_PATH`；
- 配置路径；
- 日志路径；
- 监听端口。

### 11.3 生产部署顺序

```text
备份旧二进制
备份 config.toml
停止 cc-proxy
替换二进制
更新 gpt profile 的 wire_api
启动 cc-proxy
检查启动日志
执行 E2E
```

运行中的二进制不能直接覆盖，避免：

```text
Text file busy
```

### 11.4 回滚

必须同步恢复：

```text
旧二进制
旧 config.toml
旧 gpt 模型映射
```

如果旧二进制不认识 `wire_api=responses`，临时将：

```toml
"claude-sonnet-4-6" = "deepseek-v4-pro"
```

恢复到原 Chat 模型，避免旧版本把 gpt 请求送到不兼容的 Chat 路径。

## 12. 最终验收通过目标

只有以下所有条件满足，才可以报告改造完成。

### 12.1 功能验收

- [ ] `gpt-5.6-luna` 的 `wire_api=responses` 配置生效；
- [ ] Claude Code 的 Anthropic `/v1/messages` 请求进入 cc-proxy；
- [ ] gpt 请求实际发送至 eswitch `/v1/responses`；
- [ ] xhigh 实际表现为 `reasoning.effort=xhigh`；
- [ ] Function tools 返回正确的 Anthropic `tool_use`；
- [ ] tool result 正确转换为 Responses `function_call_output`；
- [ ] 工具调用后得到最终回答；
- [ ] 非流式文本正常；
- [ ] 流式文本正常；
- [ ] 流式工具调用正常；
- [ ] 至少两轮连续 tool call 正常；
- [ ] 无需 cc-switch、proxy.py 或额外代理。

### 12.2 非回归验收

- [ ] DeepSeek 仍使用 `/v1/chat/completions`；
- [ ] GLM 仍使用原 Chat 路径；
- [ ] Kimi 仍使用原 Chat 路径；
- [ ] 现有 reasoning/tool-call 测试全部通过；
- [ ] 没有新增 Chat 400/504；
- [ ] gpt Responses 失败不会隐式降级 Chat；
- [ ] 现有 cc-proxy 服务启动、健康检查、日志行为正常。

### 12.3 Cache 验收

- [ ] `instructions` 不包含每请求变化的 `cch`；
- [ ] 工具顺序稳定；
- [ ] tool schema 序列化稳定；
- [ ] prefix fingerprint 在相同前缀下稳定；
- [ ] Responses usage 能记录 `cached_tokens`；
- [ ] 长稳定前缀第三/第四次请求命中率达到至少 90%；
- [ ] cache write/read/miss 能从 cc-proxy 日志区分；
- [ ] DeepSeek 现有 Chat cache hit 行为未下降或产生回归。

### 12.4 工程质量验收

- [ ] `cargo fmt --check` 通过；
- [ ] `cargo test` 全部通过；
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` 通过；
- [ ] 旁路验证完成后才部署生产；
- [ ] GPU 节点和 clawbot 节点均完成验证；
- [ ] 旧二进制和旧配置有可验证备份；
- [ ] reviewer 完成协议、缓存、错误处理和安全审查；
- [ ] 部署和回滚步骤均有实际命令输出证据。

## 13. 下一会话启动步骤

下一会话开始时按此顺序执行：

```bash
cd /root/projects/codewhale-proxy/source
sed -n '1,260p' GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md
git status --short --branch
git remote -v
git fetch origin
git log origin/master --oneline -3
```

然后：

1. 建立 feature 分支；
2. 记录现有 `cargo test` 基线；
3. 先写 WireApi 配置失败测试；
4. 再按垂直切片实现 request converter；
5. 每个切片执行 RED → GREEN → REFACTOR；
6. 完成单测后再做 eswitch 旁路集成；
7. 最后才考虑 GPU/clawbot 生产部署。

核心决策：

> **保留现有 cc-proxy，新增模型级 Responses transport；gpt-5.6 走 `/v1/responses`，其他模型继续走 Chat；缓存稳定化逻辑必须复用并作为独立验收项。**
