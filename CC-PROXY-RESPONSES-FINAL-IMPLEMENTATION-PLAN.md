# cc-proxy gpt-5.6 Responses Prefix/Cache 优化实施最终方案

> **For Hermes：**请按本文逐项实施，严格使用 TDD（RED → GREEN → REFACTOR），每个垂直切片完成后运行测试；不得在旁路验收前修改生产服务。
>
> **Goal：**在保留 DeepSeek/GLM/Kimi 现有 Chat 路径的前提下，修复 gpt-5.6 Responses 路径中 volatile context relocation 对历史前缀的破坏，并建立可验证的多轮 prefix/cache 观测与验收机制。
>
> **Architecture：**Claude Code 继续调用 cc-proxy 的 Anthropic `/v1/messages`；仅 gpt-5.6 的 cc-proxy→eswitch 请求走 `/v1/responses`。每轮完整重建历史是允许的，但任何已经存在的历史 input item 必须保持字节稳定；本轮动态上下文只能作为最终 synthetic input tail 追加。DeepSeek/GLM/Kimi 的 Chat 路径第一阶段不改。
>
> **Tech Stack：**Rust 2021、Axum、Reqwest、Serde/Serde JSON、Tokio、SHA-256、eswitch `http://clawbot:11434`。

---

## 1. 实施范围与当前状态

### 1.1 源码和分支

```text
源码：/root/projects/codewhale-proxy/source/
分支：feat/gpt-responses-transport
```

开始实施前必须执行：

```bash
cd /root/projects/codewhale-proxy/source
git status --short --branch
git diff --stat
git diff --check
```

当前工作树已有 Responses 改造和多份 Markdown 文档的未提交改动。不得覆盖、回滚或清理这些既有改动；只在当前分支做本文定义的增量修改。

### 1.2 必须先阅读的文件

```text
CC-PROXY-RESPONSES-CONTEXT-RECOVERY.md
GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md
RESPONSES-MULTITURN-PREFIX-CACHE-VALIDATION.md
RESPONSES-PREFIX-ALIGNMENT-CODE-REVIEW.md
RESPONSES-PREFIX-CACHE-OPTIMIZATION-PLAN.md
```

核心源码：

```text
src/responses/request.rs
src/responses/types.rs
src/responses/response.rs
src/responses/stream.rs
src/reasoning/relocate.rs
src/reasoning/prefix.rs
src/routes/messages.rs
src/client.rs
src/anthropic/converter.rs
src/config.rs
config.toml
```

### 1.3 已知基线

优化开始前重新记录：

```bash
cargo test --all-targets --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

此前基线实际通过：

```text
73 tests passed
cargo fmt --check passed
cargo clippy -D warnings passed
git diff --check passed
```

优化不能降低该基线。

---

## 2. 实测事实：为什么需要本次优化

### 2.1 多轮 input 增长是正常行为

`src/responses/request.rs:30-33` 每次完整遍历请求中的历史消息，因此正常形态是：

```text
第 1 轮：S + U1
第 2 轮：S + U1 + A1 + U2
第 3 轮：S + U1 + A1 + U2 + A2 + U3
```

总 input token 增长不代表 cache 失效。正确要求是：公共历史 item 的 wire 字节保持不变，新增内容只位于尾部。

### 2.2 relocation off 时公共历史稳定

本地 mock 捕获 cc-proxy 实际生成的 Responses request：

```text
CODEMERMAFROST_RELOCATE 未启用：
round 1→2 common_item_equal=true
round 2→3 common_item_equal=true
```

说明完整历史重建本身可以保持公共历史 item 稳定。

### 2.3 relocation on 时当前代码破坏公共历史

启用：

```bash
CODEMERMAFROST_RELOCATE=1
```

system 包含动态 env block 时：

```text
round 1→2 common_item_equal=false, first_difference=0
round 2→3 common_item_equal=false, first_difference=3
```

根因：`src/reasoning/relocate.rs:172-199` 把 appendix 追加到 `messages.last_mut()`。

```text
第 1 轮 last message = U1 → U1 被修改
第 2 轮 last message = U2 → U2 被修改
第 3 轮 last message = U3 → U3 被修改
```

因此上一轮已经存在的历史 item 会在下一轮恢复/改变，prefix 被破坏。

### 2.4 当前 fingerprint 不能证明完整 prefix

`src/responses/request.rs:57-64` 的 `cache_fingerprint` 只 hash：

```json
{
  "instructions": "...",
  "tools": [...]
}
```

不包含：

```text
input 历史、user/assistant、function_call、function_call_output、tool_choice、reasoning
```

`src/reasoning/prefix.rs:5-7` 也明确是 per-request observability、无跨请求状态。

所以现有 fingerprint 不能作为完整多轮 cache key 的证明。

### 2.5 真实 cache 基线

固定完整 `instructions + input + tools`，通过临时 cc-proxy → `http://clawbot:11434` 连续四次：

```text
round 1: input=5809, cache_read=0,    cache_creation=5806
round 2: input=5809, cache_read=5806, cache_creation=0
round 3: input=5809, cache_read=5806, cache_creation=0
round 4: input=5809, cache_read=5806, cache_creation=0
```

命中率：

```text
5806 / 5809 ≈ 99.95%
```

relocation off 的 tool history 曾出现：

```text
round 1: input=5448, cache_read=0,    cache_creation=5445
round 2: input=5479, cache_read=5445, cache_creation=31
```

这证明 append-only 多轮历史可以复用 cache。

relocation on 对照曾出现：

```text
round 1: input=5448, cache_read=0, cache_creation=5445
round 2: input=5479, cache_read=0,    cache_creation=5476
```

### 2.6 502/timeout 是独立维度

多轮真实测试中出现 HTTP 502。必须把：

```text
cache_read=0
HTTP 502/504
client timeout
```

分开记录；不能将 502 直接归因于 prefix，也不能用 Chat fallback 掩盖 Responses 错误。

---

## 3. GitHub 参考实现：必须参考，但不整体替换

本方案不是闭门造车。实现时按以下边界吸收参考项目：

| 项目 | 许可证/状态 | 参考内容 | 禁止事项 |
|---|---|---|---|
| [`mxyhi/token_proxy`](https://github.com/mxyhi/token_proxy) | Apache-2.0；调研 commit `6bed3d1ebbbb44c06833d37b34b2ebe49cc8d8a2` | Rust Anthropic↔Responses request/response 映射、tool call/result、usage、stop reason、测试结构 | 不整体替换 cc-proxy；若移植代码保留 Apache-2.0 义务；不直接采用与 EFund/eswitch 不一致的 system/header/usage 语义 |
| [`tangsipeng/openai-responses-anthropic-proxy`](https://github.com/tangsipeng/openai-responses-anthropic-proxy) | 本次未确认明确许可证 | SSE 事件状态机、function-call continuation、事件序列测试 | 不直接复制代码；第一阶段不启用 `previous_response_id` |
| [`Lokesh-Chimakurthi/rosetta-llm`](https://github.com/Lokesh-Chimakurthi/rosetta-llm) | 本次未确认明确许可证 | 独立 Responses codec、stream 事件模型、round-trip 测试边界 | 第一阶段不做全量 Canonical IR 重构 |
| [`musistudio/claude-code-router`](https://github.com/musistudio/claude-code-router) | MIT；Issue [#1372](https://github.com/musistudio/claude-code-router/issues/1372)、[#1515](https://github.com/musistudio/claude-code-router/issues/1515) | 动态 `cch` 会破坏 cache prefix；provider-specific 参数必须过滤 | 不引入完整 CCR 路由框架 |
| [`Hmbown/CodeWhale`](https://github.com/Hmbown/CodeWhale) / [`jianzhichun/permafrost`](https://github.com/jianzhichun/permafrost) | 当前 `prefix.rs`/`relocate.rs` 已注明来源 | tools 排序、billing nonce 稳定化、volatile block 检测 | 不继续采用“追加到当前 last message”的 Responses 多轮行为；该行为已被本地测试证实会改变历史 item |

采用顺序：

```text
mxyhi/token_proxy → Rust 字段映射和测试
tangsipeng + rosetta-llm → SSE/continuation/事件状态机
CCR Issues → cache/provider 风险
本地 mock + clawbot:11434 → 最终行为和验收依据
```

---

## 4. 最终架构决策

### 4.1 只优化 Responses relocation

`src/anthropic/converter.rs:63-90` 也使用现有 relocation 逻辑，但 DeepSeek/GLM/Kimi Chat 路径已有独立缓存行为。

第一阶段只改变：

```text
src/responses/request.rs
src/reasoning/relocate.rs（增加纯拆分 API）
```

Chat converter 的既有行为保持不变，除非共享内部检测函数的重构有完整 Chat 回归证据。

### 4.2 不修改已有历史 item

Responses 转换必须遵守：

```text
已有历史 message/tool item：只读
本轮动态 context：新建独立尾部 item
```

禁止把动态 appendix 追加到任何已经存在的 history message。

### 4.3 synthetic tail

目标 wire 结构：

```text
round 1: H1 + dynamic_1
round 2: H1 + A1 + U2 + dynamic_2
round 3: H1 + A1 + U2 + A2 + U3 + dynamic_3
```

`dynamic_N` 是当前请求最后追加的 synthetic user input item。建议结构：

```json
{
  "role": "user",
  "content": [
    {
      "type": "input_text",
      "text": "\\n\\n<permafrost:relocated-context>..."
    }
  ]
}
```

### 4.4 边界行为

- `SystemPrompt::Text`：第一阶段保持原行为，不扩展解析；
- 没有 volatile block：不增加 synthetic item；
- 没有历史 messages：不 relocation，不丢上下文；
- 不生成 request ID、时间戳、随机 nonce；
- 不使用 `previous_response_id`；第一阶段完整历史重放；
- 不 replay gpt encrypted reasoning。

---

## 5. 代码改造规格

### 5.1 `src/reasoning/relocate.rs`

新增纯函数：

```rust
pub fn split_volatile_system_blocks(
    system: SystemPrompt,
) -> (SystemPrompt, Vec<String>)
```

职责：

1. 只读取 system blocks；
2. 复用当前日期/UUID/hex/env marker 检测器；
3. 返回稳定 system blocks；
4. 返回 volatile 文本列表；
5. 不接收或修改 `Vec<Message>`；
6. 不产生历史副作用。

保留现有：

```rust
migrate_volatile_system_blocks(system, messages)
```

第一阶段不改变其 Chat 行为；Responses 不调用它。

保留 `stabilize_metadata()` 的 `cch → cc-proxy` 行为。

### 5.2 `src/responses/request.rs`

将当前：

```text
stabilize_metadata
→ migrate_volatile_system_blocks(system, messages)
→ append_message
```

改为：

```text
读取 system/messages
→ stabilize_metadata(system)
→ split_volatile_system_blocks(system)
→ 原样 append 全部历史 messages
→ 最后追加 synthetic volatile tail
→ tools 排序
→ 计算 hash
→ 构造 ResponsesRequest
```

建议拆分内部 API，避免单测依赖全局环境：

```rust
fn convert_request_with_relocation(
    req: &MessagesRequest,
    config: &Config,
    relocate: bool,
) -> anyhow::Result<ResponsesRequest>
```

外部 `convert_request()` 只负责：

```rust
let relocate = std::env::var("CODEMERMAFROST_RELOCATE").is_ok();
convert_request_with_relocation(req, config, relocate)
```

新增：

```rust
fn append_synthetic_context_tail(
    input: &mut Vec<Value>,
    volatile_texts: &[String],
)
```

强制条件：

- 只 append 到 `input` 尾部；
- 不修改 input 已有 item；
- 不写回 `MessagesRequest.messages`；
- 下一轮不会把上一轮 synthetic item 当作客户端历史；
- tail 不含随机字段。

### 5.3 三层 hash 观测

将当前 Responses `prefix_fingerprint` 明确视为静态形状指纹，并新增：

```text
static_prefix_hash
  = canonical({model, instructions, tools, tool_choice, reasoning})

history_prefix_hash
  = canonical(input，不含本轮 synthetic tail)

wire_input_hash
  = canonical(最终完整 input)
```

同时记录：

```text
input_item_count
history_item_count
synthetic_tail_present
input_item_types
input_tokens
cache_read_input_tokens
cache_creation_input_tokens
upstream_http_status
```

Canonical JSON：

- object key 字典序；
- array 顺序保持语义顺序；
- UTF-8；
- 紧凑 JSON；
- hash 只输出截断十六进制；
- 不输出 prompt、schema 内容、token 或 Authorization 值。

这些 hash 是诊断指标，不宣称等于上游内部 cache key。

### 5.4 `src/responses/types.rs`

第一阶段继续使用 `Vec<serde_json::Value>` 表示 Responses `input`，不为 synthetic tail 引入新的外部协议结构。

### 5.5 `src/routes/messages.rs` / `src/client.rs`

保持：

```text
gpt-5.6-luna → /v1/responses
DeepSeek/GLM/Kimi → /v1/chat/completions
```

Responses 失败不 Chat fallback。

HTTP 502/504/timeout 先增强观测，不在本次 prefix 修复中混入新重试策略。任何未来重试只能发生在首个 SSE event 发送前，且不能切换 Chat。

---

## 6. TDD 实施任务

每项必须执行：

```text
RED：写失败测试并确认失败原因正确
GREEN：最小实现
REFACTOR：保持测试通过后整理
```

### Task 0：基线与分支保护

```bash
cd /root/projects/codewhale-proxy/source
git status --short --branch
git diff --check
cargo test --all-targets --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

不得修改生产服务。

### Task 1：先写当前 bug 的失败测试

文件：

```text
src/responses/request.rs
src/reasoning/relocate.rs
```

测试名建议：

```text
relocate_on_does_not_mutate_previous_history_items
```

三轮 history 预期先观察当前：

```text
common_item_equal=false
```

运行：

```bash
cargo test --locked relocate_on_does_not_mutate_previous_history_items -- --nocapture
```

必须看到预期 RED，证明测试捕获真实 bug，而不是测试本身错误。

### Task 2：实现 system 拆分

新增 `split_volatile_system_blocks()`，不改变 Chat 旧函数输出。

测试：

```text
split_keeps_stable_blocks
split_returns_volatile_texts
split_text_system_is_unchanged
split_empty_system_is_unchanged
```

### Task 3：Responses synthetic tail

修改 Responses request converter。

测试：

```text
relocate_on_preserves_previous_history_items
relocate_appendix_is_new_tail_item
three_round_public_input_prefix_is_equal
no_volatile_block_does_not_add_tail
empty_history_does_not_drop_context
```

目标：

```text
round 1→2 common input item equal=true
round 2→3 common input item equal=true
```

### Task 4：三层 hash

测试：

```text
static_prefix_hash_excludes_input_tail
history_prefix_hash_changes_only_when_history_changes
wire_input_hash_includes_synthetic_tail
canonical_json_key_order_is_stable
```

### Task 5：Chat 回归

```bash
cargo test --locked anthropic::converter
cargo test --locked openai::converter
cargo test --locked sse
```

如果共享检测函数被重构，必须证明 Chat payload 行为没有变化。

### Task 6：完整质量门

```bash
cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

### Task 7：本地 mock wire 验证

捕获并断言三轮：

- 历史公共 input item 字节相等；
- synthetic tail 只位于最后；
- static/history/wire hash 语义正确；
- `reasoning.effort=max`；
- tools 排序；
- 不发生 Chat fallback。

### Task 8：clawbot 旁路验证

临时启动：

```bash
CODEMERMAFROST_RELOCATE=1 \
LISTEN_ADDR=127.0.0.1:11449 \
ESWITCH_URL=http://clawbot:11434 \
MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml \
DEEPSEEK_API_KEY=not-needed \
RUST_LOG=info \
./target/debug/cc-proxy
```

请求头必须有：

```http
Authorization: Bearer not-needed
Content-Type: application/json
```

测试矩阵：

1. 固定完整 request（input > 1024）连续四次；
2. 只变 user，作为对照，不单独判定为实现失败；
3. 三轮普通历史增长；
4. 三轮 tool_use/tool_result 历史增长；
5. relocation off/on；
6. 分离统计 502/504/timeout 和 cache miss；
7. 清理进程并确认 11449 无监听、11441 未触碰。

---

## 7. 验收标准

### 7.1 代码和测试

- [ ] `split_volatile_system_blocks()` 存在并有单测；
- [ ] Responses 不再调用会修改 `messages.last_mut()` 的 relocation 路径；
- [ ] synthetic tail 只位于最终 input 尾部；
- [ ] 三轮公共 input item 字节相等测试通过；
- [ ] tool_use/tool_result 历史稳定测试通过；
- [ ] 三层 hash 测试通过；
- [ ] `cargo test --all-targets --locked` 通过；
- [ ] `cargo fmt --all -- --check` 通过；
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` 通过；
- [ ] `git diff --check` 通过。

### 7.2 路由和协议

- [ ] gpt-5.6 实际走 `clawbot:11434/v1/responses`；
- [ ] `reasoning.effort=max` 实际进入上游 request；
- [ ] tools、tool continuation、SSE 正常；
- [ ] Responses 失败不隐式 fallback Chat；
- [ ] DeepSeek/GLM/Kimi 仍走 Chat；
- [ ] Claude Code 隔离入口普通、tool、多轮、stream 验证通过。

### 7.3 Prefix/cache

固定完整请求：

```text
input > 1024 tokens
instructions/input/tools 固定
连续四次
```

验收：

```text
第 2/3/4 次 cache_read_input_tokens / input_tokens >= 90%
```

多轮增长：

- [ ] 公共历史 input item 字节相等；
- [ ] `history_prefix_hash` 反映公共历史；
- [ ] 新增内容只发生在尾部；
- [ ] cache read 按上一轮稳定 prefix token 计算，而不是当前总 token；
- [ ] 502/504/timeout 与 cache miss 分开记录。

### 7.4 安全和部署

- [ ] 测试只使用临时 `127.0.0.1:11449`；
- [ ] 真实上游只使用 `http://clawbot:11434`；
- [ ] 所有测试请求带认证头；
- [ ] 不打印凭证；
- [ ] 11449 测试后无监听；
- [ ] 生产 11441 未重启、未修改；
- [ ] 优化代码形成独立 commit；
- [ ] reviewer 完成代码、协议、cache 和 Chat 回归审查；
- [ ] 未通过全部验收前不生产部署。

---

## 8. 生产发布和回滚边界

本文优化完成、测试通过和 reviewer 批准前：

```text
不修改 /etc/cc-proxy/config.toml
不重启 11441
不替换生产二进制
不修改 clawbot 服务
```

后续生产发布必须单独执行：

1. 备份生产配置和二进制；
2. 停止服务后替换二进制，避免 `Text file busy`；
3. 更新配置；
4. 启动并检查日志；
5. 先做小范围验证；
6. 观察 Chat cache 和 Responses cache；
7. 保留可验证回滚点。

回滚必须同步恢复：

```text
旧二进制
旧 config.toml
旧模型映射/wire_api
```

---

## 9. 压缩上下文后的启动提示

新会话开始时只需使用：

```text
请先完整阅读 /root/projects/codewhale-proxy/source/CC-PROXY-RESPONSES-FINAL-IMPLEMENTATION-PLAN.md、CC-PROXY-RESPONSES-CONTEXT-RECOVERY.md、RESPONSES-MULTITURN-PREFIX-CACHE-VALIDATION.md 和 RESPONSES-PREFIX-ALIGNMENT-CODE-REVIEW.md，然后在 feat/gpt-responses-transport 分支严格按最终方案执行 TDD：先为 CODEMERMAFROST_RELOCATE=1 修改历史 input 的 bug 写 RED 测试，再实现 Responses 专用 split_volatile_system_blocks + synthetic tail + 三层 prefix hash，参考 mxyhi/token_proxy/tangsipeng/rosetta-llm/CCR 但不替换 cc-proxy，保持 Chat 路径不变，先完成本地 mock、cargo 质量门和 clawbot:11434 旁路验证，绝不修改生产服务。
```

---

## 10. 最终决策

本次实施只做以下根因修复：

```text
Responses relocation 不再修改历史 message
volatile context 改为 synthetic tail
增加真实 wire input/history 观测
用多轮公共 prefix 测试证明修复
```

不做：

```text
不重写整个 cc-proxy
不新增代理
不默认 previous_response_id
不修改 Chat 路径
不以 cache miss 自动触发 Chat fallback
不把上游 502 当作 prefix bug
```

这份文件是下一阶段开发的最终实施基线。
