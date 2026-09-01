# Responses Prefix 对齐代码审查

> 日期：2026-08-05
> 审查对象：`feat/gpt-responses-transport`
> 本审查只读源码和本地测试结果，不修改源码、不访问生产业务接口。

## 1. 结论

当前实现不是完整的跨请求 Responses prefix alignment engine，而是：

```text
system billing nonce 稳定化
+ 可选 volatile system block relocation
+ tools 排序
+ per-request fingerprint 观测
+ 每次完整历史重建 Responses input
```

它能够在某些请求形状下保留历史 item，并且真实测试曾观察到 prefix cache read；但当前实现没有：

- 保存上一轮的 canonical wire input；
- 对完整 input 做 fingerprint；
- 验证公共历史 item 在多轮之间保持字节不变；
- 让 relocation appendix 跨轮次固定在不改变历史的位置。

因此“多轮 input 总长度增长”是正常的，但“当前实现已经保证多轮前缀对齐”不能从代码成立。

## 2. 多轮 input 的构建

### 2.1 完整历史每次重新转换

`src/responses/request.rs:30-33`：

```rust
let mut input = Vec::new();
for message in &messages {
    append_message(&mut input, message)?;
}
```

`src/responses/request.rs:97-133` 的 `append_message()` 将每条 Anthropic `Message` 转为 Responses input：

- 普通文本 → role/content input item（99-106 行）；
- tool_use → `function_call`（107-116 行）；
- tool_result → `function_call_output`（117-126 行）；
- thinking/redacted thinking/unknown 被跳过（127 行）。

因此多轮请求的自然形态是：

```text
round 1: S + U1
round 2: S + U1 + A1 + U2
round 3: S + U1 + A1 + U2 + A2 + U3
```

总 input token 增加不是对齐失败；对齐要求是公共历史 item 的序列化字节不变。

### 2.2 tool arguments 的序列化

`src/responses/request.rs:115` 使用 `serde_json::to_string(arguments)` 生成 function call arguments；tool result 在 125 行生成 output。历史 tool call/result 会作为后续请求的完整 input 重新构建。

## 3. 当前稳定化逻辑

### 3.1 启用条件

`src/responses/request.rs:15-29`：只有环境变量存在时才执行：

```rust
if std::env::var("CODEMERMAFROST_RELOCATE").is_ok() {
    stabilize_metadata(...)
    migrate_volatile_system_blocks(...)
}
```

因此：

- 未设置时：不执行 billing nonce 稳定化，也不执行 volatile relocation；
- 设置为任意值时：两者都执行；
- 当前代码没有默认启用。

### 3.2 billing nonce 稳定化

`src/reasoning/relocate.rs:84-122`：

- 只处理 `SystemPrompt::Blocks`；
- 查找 `x-anthropic-billing-header`；
- 将 `(cch)=...` 替换成固定 `cch=cc-proxy`。

这可以避免 Claude Code 每请求变化的 billing nonce 直接改变 instructions 前缀。

限制：

- `SystemPrompt::Text` 直接返回，不做处理（98-102 行）；
- 只匹配指定的 `cch` 结构，其他动态字段仍可能存在；
- 该逻辑不会处理历史 input 中已经存在的动态内容。

### 3.3 volatile block relocation

`src/reasoning/relocate.rs:125-208`：

- 只处理 `SystemPrompt::Blocks`（133-137 行）；
- 根据日期、UUID、hex 和环境 marker 判定 block 动态性（64-81、143-153 行）；
- 把匹配的 block 从 system 保留列表中移除；
- 把它们追加到 `messages.last_mut()`（172-199 行）；
- 新 system 由保留 block 构成（207-208 行）。

关键问题：

```text
round 1 的 last message = U1
round 2 的 last message = U2
```

因此相同历史 U1：

- round 1 会被追加 relocation appendix；
- round 2 不再是 last message，不再包含 appendix；

这会改变公共历史 item 的字节内容。它不是“只把动态内容放到尾部”的跨轮实现，而是“每次根据当前请求重新选择最后一条历史消息并修改它”。

本地 mock 证据：

```text
CODEMERMAFROST_RELOCATE=0
round 1→2 common_item_equal=true
round 2→3 common_item_equal=true

CODEMERMAFROST_RELOCATE=1 + dynamic env block
round 1→2 common_item_equal=false, first_difference=0
round 2→3 common_item_equal=false, first_difference=3
```

## 4. Fingerprint 覆盖范围

### 4.1 Responses fingerprint

`src/responses/request.rs:35-41`：

- tools 转换并按 name 排序；
- `cache_fingerprint(instructions, tools)`；
- 记录 `Responses request built`。

`src/responses/request.rs:57-64` 的 hash 输入只有：

```json
{
  "instructions": "...",
  "tools": [...]
}
```

未包含：

- `input`；
- 历史 user/assistant；
- function_call；
- function_call_output；
- tool_choice；
- reasoning；
- max_output_tokens；
- stream。

所以当前同一个 fingerprint 不能证明两个请求的完整历史前缀相同。

### 4.2 旧 prefix fingerprint

`src/reasoning/prefix.rs:5-7` 明确说明这是简化的 per-request fingerprint，没有跨请求状态。

`src/reasoning/prefix.rs:18-43` 只 hash：

- system prompt；
- 排序后的 tool names。

它同样不能证明 Responses 完整 input prefix。

## 5. 路由与上游调用边界

`src/routes/messages.rs:61-83`：

- 先将 Claude 模型映射为 upstream model；
- 按 `wire_api` 选择 Responses；
- Responses 调用走 `responses_completion()` 或 `responses_completion_stream()`；
- 该分支没有调用 Chat fallback。

`src/client.rs:108-151`：

- Responses 上游路径是 `{base_url}/v1/responses`；
- 请求使用当前 client 的 Authorization header；
- 非 2xx 返回错误。

这部分路由符合目标，但 cache prefix 对齐属于请求转换层，不能由 HTTP client 自动解决。

## 6. 最小优化方案

### 6.1 修正 relocation 位置

不要再修改 `messages.last_mut()`。建议：

1. 先把所有历史 messages 原样转换为 input；
2. 将动态 context 放入一个新的最后 synthetic user input item；
3. 新 item 只属于当前请求，不回写/伪装为历史消息；
4. 下一轮历史重放时，旧历史 item 的字节保持不变；
5. 每轮动态内容可变化，但只影响当前请求尾部。

目标形态：

```text
round 1: H1 + dynamic_1
round 2: H1 + A1 + U2 + dynamic_2
round 3: H1 + A1 + U2 + A2 + U3 + dynamic_3
```

### 6.2 增加真实 prefix 观测

至少增加以下 hash：

```text
static_prefix_hash
  instructions + stable tools

history_prefix_hash
  本轮动态 synthetic item 之前的完整 input

wire_input_hash
  最终完整 input
```

也记录：

```text
input_item_count
input_item_types
input_tokens
cache_read_input_tokens
cache_creation_input_tokens
```

不要把当前 `prefix_fingerprint` 的语义写成完整 cache key。

### 6.3 回归测试

至少增加：

1. `relocate_on_preserves_previous_history_items`；
2. `relocate_appendix_is_new_tail_item`；
3. 三轮历史公共 item 字节相等；
4. tool_use/tool_result 历史序列化稳定；
5. full input hash 随新增尾部变化但公共 prefix hash 保持；
6. mock upstream 捕获三轮 Responses 请求；
7. 真实 clawbot 旁路固定完整请求、tool continuation、多轮增长分别测试。

## 7. 结论

当前代码已实现“部分稳定化和观测”，没有实现完整的跨请求多轮 prefix alignment。多轮 input 增长本身正常；真正需要修复的是 relocation 修改历史 last message，以及 fingerprint 未覆盖完整 input。任何生产优化前应先完成上述最小修复设计和回归测试，不能把上游 502 或一次 cache miss 直接归因于单一原因。
