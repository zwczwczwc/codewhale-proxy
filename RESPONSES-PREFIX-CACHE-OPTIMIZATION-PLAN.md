# cc-proxy Responses 多轮 Prefix/Cache 最终优化方案

> **For Hermes：**下一阶段只实现本文定义的 Responses prefix 修复；先按 TDD 完成本地 mock 回归，再做 `clawbot:11434` 旁路验证，未经单测、评审和旁路验收不得修改生产服务。
>
> **Goal：**在保持 DeepSeek/GLM/Kimi 现有 Chat 路径不变的前提下，修复 gpt-5.6 Responses 路径中 volatile context relocation 对历史前缀的破坏，并建立可证明的多轮 prefix/cache 观测与验收机制。
>
> **Architecture：**Claude Code 仍调用 cc-proxy 的 Anthropic `/v1/messages`；只有 gpt-5.6 的 cc-proxy→eswitch 请求使用 `/v1/responses`。Responses 路径每轮重建完整历史，但不修改已经存在的历史 input item；本轮动态环境内容作为最终 synthetic input tail 追加。现有 Chat 路径先保持行为不变，单独进行回归和后续评估。
>
> **Tech Stack：**Rust 2021、Axum、Reqwest、Serde/Serde JSON、Tokio、SHA-256、eswitch `http://clawbot:11434`。

---

## 1. 本方案的依据

### 1.1 实施对象

源码目录：

```text
/root/projects/codewhale-proxy/source/
```

当前开发分支：

```text
feat/gpt-responses-transport
```

当前已有 Responses 实现，但尚未实施本文优化。当前工作树已有未提交改动；优化开始前必须先检查 Git 状态，不得覆盖、回滚或混入无关改动。

核心文件：

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
config.toml
```

### 1.2 必须阅读的背景文件

```text
CC-PROXY-RESPONSES-CONTEXT-RECOVERY.md
GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md
RESPONSES-MULTITURN-PREFIX-CACHE-VALIDATION.md
RESPONSES-PREFIX-ALIGNMENT-CODE-REVIEW.md
```

本方案是结合前两份实施基线和后两份实际验证/代码审查结果形成的最终优化方案。

### 1.3 GitHub 参考项目与采用边界

本方案不是闭门设计；实现阶段必须参考此前调研过的开源项目，但只能吸收经过本地事实验证后适合当前 cc-proxy 的部分，不能整体替换当前服务或未经许可证确认直接复制代码。

| 项目 | 许可证/状态 | 在本方案中的用途 | 不采用的部分 |
|---|---|---|---|
| [`mxyhi/token_proxy`](https://github.com/mxyhi/token_proxy) | Apache-2.0；调研 commit `6bed3d1ebbbb44c06833d37b34b2ebe49cc8d8a2` | **主参考**：Rust Anthropic↔Responses request/response 映射、`tool_use↔function_call`、`tool_result↔function_call_output`、usage/stop reason 和测试结构 | 不整体替换 cc-proxy；不直接采用其与 EFund/eswitch 不一致的 system、header、usage 语义；若移植代码须保留 Apache-2.0 义务 |
| [`tangsipeng/openai-responses-anthropic-proxy`](https://github.com/tangsipeng/openai-responses-anthropic-proxy) | 本次调研未确认明确许可证 | **协议/SSE 参考**：Responses SSE 事件状态机、function-call continuation、`previous_response_id` fallback、事件序列测试 | 不直接复制代码；第一阶段不启用 `previous_response_id` |
| [`Lokesh-Chimakurthi/rosetta-llm`](https://github.com/Lokesh-Chimakurthi/rosetta-llm) | 本次调研未确认明确许可证 | **codec/测试参考**：独立 Responses stream codec、统一事件模型、round-trip 测试边界 | 第一阶段不把 cc-proxy 全量重构为 Canonical IR |
| [`musistudio/claude-code-router`](https://github.com/musistudio/claude-code-router) | MIT；重点 Issue [#1372](https://github.com/musistudio/claude-code-router/issues/1372)、[#1515](https://github.com/musistudio/claude-code-router/issues/1515) | **风险参考**：动态 `cch`/billing header 会破坏 cache prefix；provider 不支持的 `thinking/reasoning` 字段不能无条件转发 | 不引入其完整路由框架 |
| [`Hmbown/CodeWhale`](https://github.com/Hmbown/CodeWhale) / [`jianzhichun/permafrost`](https://github.com/jianzhichun/permafrost) | 当前 `src/reasoning/prefix.rs`、`relocate.rs` 已注明来源 | **局部算法参考**：工具排序、billing nonce 稳定化、volatile block 识别 | 不把原有“追加到当前 last message”的行为直接用于 Responses 多轮历史；本次实测已证明该位置会改变已存在历史 item |

采用顺序：

```text
mxyhi/token_proxy
  → 请求/响应字段映射和 Rust 测试

tangsipeng + rosetta-llm
  → SSE 状态机、tool continuation、事件测试

CCR Issue #1372/#1515
  → cache prefix 和 provider 字段过滤风险

本仓库真实 mock/旁路证据
  → 最终决定 relocation synthetic tail 和验收标准
```

外部项目只能提供模式参考；本次最终方案的决定性依据是本仓库实际捕获的 `CODEMERMAFROST_RELOCATE=1` 多轮公共 item 不相等，以及 `clawbot:11434` 上固定完整请求约 99.95% cache read 的真实结果。

### 1.4 正确测试上游和安全边界

开发旁路的正确上游：

```text
http://clawbot:11434
```

`clawbot` 当前解析到：

```text
100.64.0.1
```

不得将本机以下地址当作本次 eswitch：

```text
http://127.0.0.1:11434
```

访问 clawbot 时必须使用：

```http
Authorization: Bearer not-needed
Content-Type: application/json
```

缺少 Authorization 得到的：

```text
HTTP 401: Missing Authentication header
```

只能解释为测试请求认证缺失，不能解释为 cache、Responses 或 cc-proxy 能力失败。

临时 cc-proxy 只允许：

```text
127.0.0.1:11449
```

测试结束必须：

```text
SIGTERM 临时进程
确认 11449 无监听
确认生产 11441 仍监听且未重启/修改
```

不得修改：

```text
/etc/cc-proxy/
systemd cc-proxy 服务
生产端口 11441
生产二进制
```

---

## 2. 实测事实与最终判断

## 2.1 多轮 input 变长不是问题本身

当前 `src/responses/request.rs:30-33` 每次请求都会完整遍历 Anthropic history：

```rust
let mut input = Vec::new();
for message in &messages {
    append_message(&mut input, message)?;
}
```

所以正常多轮形态是：

```text
第 1 轮：S + U1
第 2 轮：S + U1 + A1 + U2
第 3 轮：S + U1 + A1 + U2 + A2 + U3
```

正确的 prefix cache 目标不是让总 input token 数不变，而是：

```text
已存在历史 item 的序列化字节不变
新内容只追加到已有历史之后
```

## 2.2 relocation off 时公共历史稳定

本地 mock 捕获 cc-proxy 实际发出的 Responses JSON，`CODEMERMAFROST_RELOCATE` 未启用时：

```text
round 1 → round 2:
  common_item_count=1
  common_item_equal=true
  first_difference=None

round 2 → round 3:
  common_item_count=3
  common_item_equal=true
  first_difference=None
```

这证明完整历史重建本身可以保持公共历史 item 字节稳定。

## 2.3 relocation on 时当前实现破坏公共历史

启用：

```bash
CODEMERMAFROST_RELOCATE=1
```

并在 system 中加入动态 env block 后：

```text
round 1 → round 2:
  common_item_count=1
  common_item_equal=false
  first_difference=0

round 2 → round 3:
  common_item_count=4
  common_item_equal=false
  first_difference=3
```

这是本地 mock 对 cc-proxy 实际转换结果的结构比较，不是推测。

根因：

`src/reasoning/relocate.rs:172-199` 把 relocation appendix 追加到当前请求的 `messages.last_mut()`：

```text
第 1 轮 last message = U1
第 2 轮 last message = U2
第 3 轮 last message = U3
```

于是：

```text
第 1 轮：U1 + appendix
第 2 轮：U1 + ... + U2 + appendix
```

同一个历史 U1 在两轮中的 wire 内容不同，公共 prefix 被破坏。

## 2.4 当前 fingerprint 不是完整 cache key 证明

`src/responses/request.rs:57-64` 的 `cache_fingerprint` 只 hash：

```json
{
  "instructions": "...",
  "tools": [...]
}
```

它不包含：

```text
input 历史
assistant/user 历史
function_call
function_call_output
tool_choice
reasoning
```

`src/reasoning/prefix.rs:5-7` 也明确说明当前实现是：

```text
Per-request prefix fingerprint computation
No cross-request state
```

因此当前 fingerprint 只能作为静态请求形状的观测值，不能证明完整多轮历史前缀相同。

## 2.5 `CODEMERMAFROST_RELOCATE` 当前不是默认开启

`src/responses/request.rs:15-29`：

```rust
if std::env::var("CODEMERMAFROST_RELOCATE").is_ok() {
    stabilize_metadata(...)
    migrate_volatile_system_blocks(...)
}
```

结论：

- 环境变量未设置：Responses 不执行当前稳定化/relocation；
- 环境变量设置为任意值：两者执行；
- 当前没有默认启用。

## 2.6 真实 cache 结果

### 固定完整请求

通过临时 cc-proxy → `http://clawbot:11434`，稳定 `instructions + input + tools` 连续四次：

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

结论：上游 Responses cache 和 cc-proxy→clawbot 链路正常。

### tool history append-only 对照

在 relocation off 的历史增长场景中曾得到：

```text
round 1: input=5448, cache_read=0,    cache_creation=5445
round 2: input=5479, cache_read=5445, cache_creation=31
```

这证明多轮历史增长并不天然破坏 cache；新增 31 tokens 时，上一轮 5445 tokens 被复用。

### relocation on 对照

对应场景曾得到：

```text
round 1: input=5448, cache_read=0, cache_creation=5445
round 2: input=5479, cache_read=0, cache_creation=5476
```

结合本地公共 item 不相等证据，当前最强代码级问题是 relocation 修改了历史 input。

### 502 单独处理

多轮真实测试中还出现过：

```text
HTTP 502
```

502/timeout 属于上游/LB/网关可用性维度，不能和 cache miss 混为一个结论。优化方案必须分别记录：

```text
request_success
cache_read/cache_write
upstream_502/504/timeout
```

不能用 Chat fallback 掩盖 Responses 上游错误。

---

## 3. 最终设计决策

### 3.1 只修复 Responses relocation，不先改变 Chat 生产路径

当前同一个 `migrate_volatile_system_blocks()` 也被 Chat converter 使用，见：

```text
src/anthropic/converter.rs:63-90
```

DeepSeek/GLM/Kimi Chat 路径已有高 cache hit 历史，用户要求最小化变更。因此第一阶段：

- 新增 Responses 专用的非破坏性 volatile extraction；
- Responses 使用新的 synthetic tail 方案；
- 保留 Chat converter 当前行为不变；
- Chat relocation 另做独立基线和回归，不能在本次 Responses 修复中顺手改变。

这不是忽略 Chat 问题，而是为了避免一次改造同时改变两条协议路径。

### 3.2 不修改已经存在的历史 input item

Responses 路径必须满足：

```text
已有历史 message/tool item：只读，不追加，不重写
本轮动态 context：单独创建新 tail item
```

禁止：

```rust
messages.last_mut().content += appendix
```

### 3.3 动态 context 作为 synthetic tail input

目标形态：

```text
round 1: H1 + dynamic_1
round 2: H1 + A1 + U2 + dynamic_2
round 3: H1 + A1 + U2 + A2 + U3 + dynamic_3
```

其中：

- `H1` 等历史 item 在每轮保持字节不变；
- `dynamic_N` 每轮可变化，但只位于本轮最终 input 尾部；
- 下一轮不会把上一轮 synthetic item 当作客户端历史重放，因此不会修改既有历史。

建议 synthetic item 使用 Responses message 形状：

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

保留现有 `<permafrost:relocated-context>` 语义包装，避免模型误解上下文来源。

### 3.4 空 history 和纯文本 system 的边界

为最小化语义变化：

- `SystemPrompt::Text` 暂不扩展动态 block 解析，保持当前行为；
- system 为 blocks 但没有 volatile block：不追加 synthetic item；
- 没有历史 messages 时：不做 relocation，保留 volatile 内容在 system，避免凭空生成 user turn；
- 只有存在至少一条 history message 且检测到 volatile blocks 时，才生成 synthetic tail。

---

## 4. 具体代码改造方案

## 4.1 `src/reasoning/relocate.rs`

新增纯函数，建议命名：

```rust
pub fn split_volatile_system_blocks(
    system: SystemPrompt,
) -> (SystemPrompt, Vec<String>)
```

职责：

1. 只读取 system blocks；
2. 按当前已有 detector 判断 volatile blocks；
3. 返回稳定 system blocks；
4. 返回待追加的 volatile 文本列表；
5. 不接收、不修改 `Vec<Message>`；
6. 不产生任何历史消息副作用。

保留现有：

```rust
migrate_volatile_system_blocks(system, messages)
```

第一阶段不改变其 Chat 行为，以保护现有 Chat 路径；Responses 不再调用它。

可选内部重构：让旧函数复用相同的 `split_volatile_system_blocks()` 检测逻辑，但输出行为必须保持原样，并需要完整 Chat 回归。

保留：

```rust
stabilize_metadata()
```

其当前 `cch → cc-proxy` 行为不变。

## 4.2 `src/responses/request.rs`

将当前：

```rust
stabilize_metadata()
→ migrate_volatile_system_blocks(system, messages)
→ append_message()
```

改为：

```text
1. 读取 system/messages；
2. 如果 relocation flag 未启用，保持现有路径；
3. 如果 relocation flag 已启用：
   a. stabilize_metadata(system)
   b. split_volatile_system_blocks(system)
   c. 原样 append 所有历史 messages 到 input
   d. 将 volatile 文本拼为一个 synthetic tail input item
4. tools 排序；
5. 计算观测 hash；
6. 序列化 ResponsesRequest
```

建议内部拆分：

```rust
fn convert_request_with_relocation(
    req: &MessagesRequest,
    config: &Config,
    relocate: bool,
) -> anyhow::Result<ResponsesRequest>
```

外部 `convert_request()` 只负责读取环境变量并调用内部函数。这样测试不依赖并行测试中的全局环境变量：

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

要求：

- synthetic tail 只能追加在 input 最后；
- 不修改 `input` 中已有 item；
- 不把 synthetic tail 写回 `MessagesRequest.messages`；
- 下一轮历史重放不会包含上一轮 synthetic tail；
- 不向 tail 写入 request ID、时间戳或随机 nonce。

## 4.3 观测 hash 重新定义

保留旧字段名以兼容现有日志，但明确其语义为静态形状：

```text
prefix_fingerprint = static_prefix_fingerprint
```

新增三个 hash：

```text
static_prefix_fingerprint
  = canonical({model, instructions, tools, tool_choice, reasoning})

history_prefix_fingerprint
  = canonical(input，不含本轮 synthetic tail)

wire_input_fingerprint
  = canonical(最终完整 input)
```

同时记录：

```text
input_item_count
history_item_count
synthetic_tail_present
input_item_types
```

响应完成时记录：

```text
input_tokens
cache_read_input_tokens
cache_creation_input_tokens
upstream_http_status
```

### Canonical JSON 要求

hash 不能依赖不可控的 JSON map 插入顺序。新增递归 canonicalization：

- object key 按字典序；
- array 顺序保持原语义顺序；
- 字符串 UTF-8；
- 紧凑分隔符；
- hash 只输出截断后的十六进制摘要；
- 日志禁止输出 prompt、tool schema 内容和凭证。

这三个 hash 是诊断指标，不宣称等于上游内部 cache key。

## 4.4 `src/responses/types.rs`

第一阶段无需改变 Responses API 外部结构，只需要支持 synthetic input item 的内部构造。

可以继续使用：

```rust
Vec<serde_json::Value>
```

因为当前实现已经使用 Value 保存 Responses input item。

## 4.5 `src/routes/messages.rs` 与 `src/client.rs`

第一阶段不改变路由选择和 Chat fallback 策略：

- `gpt-5.6-luna → /v1/responses`；
- DeepSeek/GLM/Kimi → `/v1/chat/completions`；
- Responses 失败不切换 Chat。

对 HTTP 502/504：

1. 先增加结构化 `upstream_http_status` 观测；
2. 将 502/504/timeout 和 cache miss 分开统计；
3. 不在 prefix 修复中同时引入重试策略；
4. 如果 502 持续，再建立独立的 Responses upstream availability 任务；
5. 任何重试必须只发生在尚未向客户端发送首个 SSE event 前，且不能 Chat fallback。

---

## 5. TDD 实施顺序

### Task 1：先写失败测试证明当前 bug

文件：

```text
src/responses/request.rs
src/reasoning/relocate.rs
```

测试：

```text
relocate_on_does_not_mutate_previous_history_items
```

构造三轮 history，当前实现应出现：

```text
round 1→2 common_item_equal=false
```

先运行：

```bash
cargo test --locked relocate_on_does_not_mutate_previous_history_items -- --nocapture
```

预期：RED，证明测试确实捕获当前 bug。

### Task 2：增加 `split_volatile_system_blocks`

只实现 system block 拆分，不改 Chat 旧函数行为。

测试：

```text
split_keeps_stable_blocks
split_returns_volatile_texts
split_text_system_is_unchanged
split_empty_system_is_unchanged
```

### Task 3：Responses 使用 synthetic tail

修改 `src/responses/request.rs`。

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

### Task 4：hash 观测

测试：

```text
static_prefix_hash_excludes_input_tail
history_prefix_hash_changes_only_when_history_changes
wire_input_hash_includes_synthetic_tail
canonical_json_key_order_is_stable
```

确认日志不包含原始 prompt/token。

### Task 5：Chat 回归

不改变 Chat 路径行为，执行：

```bash
cargo test --locked anthropic::converter
cargo test --locked openai::converter
cargo test --locked sse
```

如果选择重构旧 `migrate_volatile_system_blocks()` 的内部检测代码，必须在这一步证明 Chat 输出 payload 没有变化。

### Task 6：完整质量门

```bash
cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

### Task 7：本地 mock upstream

使用 mock 捕获：

- 三轮 Responses request；
- `instructions`；
- input item types；
- history/wire hash；
- synthetic tail 位置；
- `reasoning.effort=xhigh`；
- tools 排序；
- 不发生 Chat fallback。

### Task 8：clawbot 旁路

启动：

```bash
CODEMERMAFROST_RELOCATE=1 \
LISTEN_ADDR=127.0.0.1:11449 \
ESWITCH_URL=http://clawbot:11434 \
MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml \
DEEPSEEK_API_KEY=not-needed \
RUST_LOG=info \
./target/debug/cc-proxy
```

请求必须带：

```http
Authorization: Bearer not-needed
Content-Type: application/json
```

测试矩阵：

1. 固定完整请求，input > 1024 tokens，连续 4 次；
2. 只变化 user，作为对照，不把全请求不命中直接判为 bug；
3. 三轮正常文本历史增长；
4. 三轮 tool_use/tool_result 历史增长；
5. relocation off/on 对照；
6. 记录 502/504/timeout 独立计数；
7. 测试后停止进程并确认 11449 无监听。

---

## 6. Cache 验收标准

### 6.1 固定完整请求

条件：

```text
instructions + input + tools 固定
input_tokens > 1024
连续 4 次
```

通过条件：

```text
第 2/3/4 次 cache_read_input_tokens / input_tokens >= 90%
```

当前基线已达到：

```text
5806 / 5809 ≈ 99.95%
```

### 6.2 多轮增长

不使用当前总 input 作为唯一分母。

对于第 N 轮，比较：

```text
cache_read_input_tokens
    / 第 N-1 轮稳定 wire input token 数
```

同时要求：

```text
公共历史 input item 字节完全相等
```

如果公共历史不等，先判为 cc-proxy 转换问题；如果公共历史相等但 cache_read 仍为 0，再调查上游/LB/cache worker。

### 6.3 502/timeout

单独记录：

```text
HTTP 502/504
client timeout
Responses parse error
cache_read=0
```

规则：

- HTTP 非 2xx 不进入 cache 命中率计算；
- 不把 timeout 当作 cache miss；
- 不把 cache miss 当作 HTTP 失败；
- 不用 Chat fallback 掩盖失败。

---

## 7. 生产发布边界

本文优化完成前：

```text
不发布生产
不修改 /etc/cc-proxy/config.toml
不重启 11441
不替换生产二进制
```

完成后仍需：

1. reviewer 审查 Responses converter、relocation、hash、Chat 非回归；
2. 旁路测试全部通过；
3. 形成 commit；
4. 备份生产二进制和配置；
5. 单独批准灰度；
6. 观察生产 Chat cache hit 和 Responses cache 指标；
7. 准备可验证回滚。

回滚时必须同时恢复：

```text
旧二进制
旧 config.toml
旧模型映射/wire_api
```

---

## 8. 最终判断

本次实测已经排除：

```text
“只是对话轮次不够”是主要原因
“完整历史增长天然不能命中”是根因
```

本次实测确认：

```text
固定完整请求可以约 99.95% 命中
relocation off 时公共历史可保持字节稳定
relocation on 时当前实现会修改历史 input item
当前 fingerprint 不覆盖完整 input
上游偶发 502 必须独立处理
```

因此最终优化顺序必须是：

```text
1. 先修复 Responses relocation 的历史 mutation
2. 增加 synthetic tail
3. 增加 history/wire prefix hash 观测
4. 用本地 mock 证明三轮公共 prefix 字节不变
5. 用 clawbot:11434 做多轮 cache 旁路复验
6. 将 502/timeout 与 cache miss 分开统计
7. 通过 reviewer 和完整质量门后再讨论生产部署
```

本文只定义方案，未实施上述源码优化。
