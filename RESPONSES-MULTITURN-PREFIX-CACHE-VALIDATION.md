# Responses 多轮 Prefix / Cache 验证报告

> 日期：2026-08-05
> 范围：`feat/gpt-responses-transport` 开发分支；不修改源码、不修改生产配置、不重启生产服务。
> 正确上游：`http://clawbot:11434`（host 解析到 `100.64.0.1`）。本机 `127.0.0.1:11434` 不属于本次测试目标。
> 认证：测试请求使用非敏感占位头 `Authorization: Bearer not-needed`；本报告不包含任何真实凭证。

## 1. 结论摘要

1. **多轮完整 input 变长是正常的**：完整历史重放会形成 `S+U1` → `S+U1+A1+U2` → `S+U1+A1+U2+A2+U3`。
2. **在 `CODEMERMAFROST_RELOCATE` 未启用时，当前本地转换能保持历史 input item 的字节/结构不变**。本地 mock 捕获结果显示：第一轮到第二轮的公共 item 相等，第二轮到第三轮的公共 item 也相等。
3. **在 `CODEMERMAFROST_RELOCATE=1` 且存在动态 system/env block 时，当前实现会破坏多轮历史前缀**。本地 mock 捕获结果显示公共历史 item 在轮次之间不相等，第一处差异出现在 item 0，第二轮开始就会影响已有前缀。
4. 根因位于 `migrate_volatile_system_blocks()`：它把动态 block **追加到“当前请求的最后一条消息”**。随着新 user turn 进入，上一轮曾被追加内容的消息不再是最后一条，于是同一历史消息在下一轮又恢复为原始形态，造成字节变化。
5. 当前 Responses fingerprint 只覆盖 `instructions + tools`，不覆盖完整 `input`、历史 tool call/result、`tool_choice` 或 reasoning；它是观测指纹，不是完整多轮 prefix 对齐证明。
6. 真实上游验证显示：固定完整 Responses 请求可以命中；tool-call 历史在 relocation 关闭时曾观察到上一轮输入的大部分缓存复用；开启 relocation 的路径没有稳定命中。真实上游还出现间歇性 HTTP 502，必须与前缀问题分开分析。
7. 最小优化方向：**不再修改任何已经发出的历史 message；将本轮动态 relocation 内容作为独立的、最后追加的 synthetic input item 发送**，并增加完整 input/history prefix hash 观测与多轮回归测试。

## 2. 测试安全边界

- 临时 cc-proxy 只监听 `127.0.0.1:11449`。
- 上游使用 `ESWITCH_URL=http://clawbot:11434`。
- 临时进程结束后检查 `11449` 无监听。
- 生产 `11441` 仍由原有 cc-proxy 监听，未重启、未修改。
- 没有读取、打印或写入真实 token。

## 3. 纯本地转换捕获结果

测试使用本地 mock upstream 捕获 cc-proxy 发出的 Responses JSON，因此可以比较实际序列化前的结构；请求不会访问真实上游。

### 3.1 `CODEMERMAFROST_RELOCATE` 未启用

捕获的三轮 input：

| 轮次 | input item 数 | item 类型 | instructions hash | input hash |
|---:|---:|---|---|---|
| 1 | 1 | `user` | `a9c5b6880f36c5f9` | `5b5396bc2c36395d` |
| 2 | 3 | `user,function_call,function_call_output` | `a9c5b6880f36c5f9` | `1873db0ad0529bce` |
| 3 | 5 | `user,function_call,function_call_output,assistant,user` | `a9c5b6880f36c5f9` | `47a31dda91bcfb1c` |

公共前缀比较：

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

结论：没有 relocation 时，历史 item 是追加式增长，已有公共 item 保持一致。

### 3.2 `CODEMERMAFROST_RELOCATE=1`

测试 system 中包含动态 env block。捕获结果：

| 轮次 | input item 数 | item 类型 | instructions hash | input hash |
|---:|---:|---|---|---|
| 1 | 1 | `user` | `d6aa699f508d6465` | `0f3364cf4881dea2` |
| 2 | 4 | `user,function_call,function_call_output,user` | `d6aa699f508d6465` | `255ea0c07a9d6c85` |
| 3 | 5 | `user,function_call,function_call_output,assistant,user` | `d6aa699f508d6465` | `a0143cb036c49470` |

公共前缀比较：

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

这不是推测，而是本地 mock 对 cc-proxy 实际 Responses 请求转换结果的结构比较。

解释：

- round 1 只有一个 user message，relocation appendix 被追加到该 message；
- round 2 的输入历史重新从 Anthropic messages 转换，原 round-1 user message 没有 appendix，而新 appendix 被追加到 round-2 的最后 user message；
- 因此 round-1 的第一个 input item 在 round-2 中发生变化；
- round 2 → round 3 时，上一轮最后的 user message 不再是当前最后 user message，因此公共位置 3 发生变化。

### 3.3 instructions 变化

同一动态 system block：

```text
relocate off: instructions_chars=8044, hash=a9c5b6880f36c5f9
relocate on : instructions_chars=7920, hash=d6aa699f508d6465
```

这是预期的 system relocation 结果：动态 block 从 instructions 移走。但当前 appendix 的挂载位置会随“最后一条消息”变化，导致历史 input 不稳定。

## 4. 真实 clawbot eswitch 旁路结果

测试请求均通过临时 cc-proxy → `http://clawbot:11434`，请求头包含认证占位头。

### 4.1 固定完整请求

同一 system/instructions、同一完整 input、同一 tools 连续四次：

```text
round 1: HTTP 200, input_tokens=5809, cache_read=0,    cache_creation=5806
round 2: HTTP 200, input_tokens=5809, cache_read=5806, cache_creation=0
round 3: HTTP 200, input_tokens=5809, cache_read=5806, cache_creation=0
round 4: HTTP 200, input_tokens=5809, cache_read=5806, cache_creation=0
```

命中率：

```text
5806 / 5809 ≈ 99.95%
```

这证明当前链路和上游 Responses cache 均可工作。

### 4.2 只改变 user

固定 instructions/tools，只改变 user 文本，四次均为：

```text
cache_read=0
cache_creation≈input_tokens-3
```

这一场景不能简单解释为“轮次不够”。它说明上游当前缓存匹配至少受到完整 input 形状或 token block 边界影响；不能只依据 `instructions + tools` 相同就推断应该命中。

### 4.3 完整历史增长

一组真实增长历史：

```text
round 1: U1
round 2: U1 + A1 + U2
round 3: U1 + A1 + U2 + A2 + U3
```

结果：

```text
round 1: HTTP 200, input_tokens=5987, cache_read=0, cache_creation=5984
round 2: HTTP 200, input_tokens=6018, cache_read=0, cache_creation=6015
round 3: HTTP 502
```

这组结果同时暴露两个问题：

1. 当前测试配置中历史增长路径没有形成稳定的 cache read 证据；
2. 上游/网关出现 HTTP 502，不能把该次失败全部归因于 prefix 对齐。

### 4.4 tool-call 历史增长对照

在另一组连续测试中，关闭 relocation 时观察到：

```text
round 1: input_tokens=5448, cache_read=0,    cache_creation=5445
round 2: input_tokens=5479, cache_read=5445, cache_creation=31
round 3: HTTP 502
```

round 2 的结果非常重要：新增 31 tokens 时复用了前一轮 5445 tokens，说明**完整历史追加式增长可以被上游 prefix cache 复用**。

开启 relocation 的对应路径曾观察到：

```text
round 1: input_tokens=5448, cache_read=0,    cache_creation=5445
round 2: input_tokens=5479, cache_read=0,    cache_creation=5476
round 3: HTTP 502
```

结合本地 mock 的公共 item 不相等证据，relocation 是该路径失去 prefix reuse 的明确代码级嫌疑；但由于两次真实测试会话受上游缓存/LB状态影响，不能仅凭这两组就断言它是所有 HTTP 502 的唯一原因。

## 5. 当前实现的事实边界

### 已经实现

- tools 按名称排序：`src/responses/request.rs:35-39`；
- `x-anthropic-billing-header` 中的 `cch` nonce 稳定化：`src/reasoning/relocate.rs:84-122`；
- 动态 env block relocation：`src/reasoning/relocate.rs:125-208`；
- 完整 Anthropic message history 转为 Responses `input`：`src/responses/request.rs:30-33,97-133`；
- tool_use/tool_result 转为 function_call/function_call_output：`src/responses/request.rs:107-126`。

### 当前缺口

1. relocation 直接修改当前请求的最后一条历史 message；
2. Responses `cache_fingerprint` 只 hash instructions/tools：`src/responses/request.rs:40,57-64`；
3. 旧的 `src/reasoning/prefix.rs` 也只 hash system prompt 和 tool names，且文件注释写明是 per-request observability、无跨请求状态；
4. 没有记录完整 input 的 digest、公共历史 digest、当前新增尾部 digest；
5. 没有测试 `CODEMERMAFROST_RELOCATE=1` 时多轮公共历史 item 保持相等；
6. 真实上游存在偶发 HTTP 502，需要独立记录并与 cache miss 分离。

## 6. 结论：轮次、长度、前缀和上游因素

| 假设 | 当前证据判断 |
|---|---|
| 只是对话轮次不够 | 不成立为主要解释：固定完整请求第 2 次已命中；tool 历史 round 2 也曾命中 |
| prompt 长度不够 | 确实存在阈值，但本报告测试使用约 5.4K–6.0K tokens，长度足够触发 cache |
| 完整历史增长天然不能命中 | 不成立：relocation off 的 tool 历史 round 2 复用 5445 tokens |
| system/tools fingerprint 相同就应命中 | 不成立：实际 cache key 还受到完整 input/序列化/block 边界影响 |
| relocation 会破坏历史前缀 | 有明确本地结构证据；开启 relocation 后公共 input item 不相等 |
| 所有失败都来自 cc-proxy | 不成立：真实测试出现 HTTP 502，且 LB/cache 状态需独立排查 |

## 7. 优化方案设计输入

### 最小必要修复方向（尚未实施）

将 relocation 从“修改当前最后一条历史 message”改为：

```text
1. 先把收到的历史 messages 原样转换为 input；
2. 不修改任何已存在的历史 input item；
3. 把本轮 volatile context 作为一个新的、最后追加的 synthetic user input item；
4. 本轮真实 user/tool history 保持原 item 顺序和字节；
5. 下一轮重新接收历史时，不会因为 appendix 从上一条消息迁移到另一条消息而改变旧前缀。
```

这样可保持：

```text
round 1: H1 + dynamic_1
round 2: H1 + A1 + U2 + dynamic_2
round 3: H1 + A1 + U2 + A2 + U3 + dynamic_3
```

其中 `H1` 等已存在历史 item 保持不变，动态内容只在尾部变化。

### 观测改造

建议区分三个 hash：

```text
static_prefix_hash
  = instructions + stable tools

history_prefix_hash
  = 本轮新增动态 appendix 之前的完整历史 input

wire_input_hash
  = 最终完整 Responses input
```

当前 `prefix_fingerprint` 不应继续被解释为完整 cache key。

### 必须新增的回归测试

1. `relocate_on_preserves_previous_history_items`：三轮历史转换，逐轮比较公共 input item；
2. `relocate_appendix_is_new_tail_item`：确认动态 appendix 不修改已有 user/tool item；
3. `responses_history_digest_excludes_current_dynamic_tail`；
4. `responses_fingerprint_includes_relevant_wire_shape`；
5. `tool_use_tool_result_history_is_byte_stable`；
6. mock upstream 捕获三轮请求并断言公共 prefix 相等；
7. 真实 clawbot 旁路：固定完整请求、tool continuation、多轮增长分别测试，记录 HTTP 502 与 cache miss 的独立统计。

## 8. 现场清理

本次诊断结束后的现场检查：

```text
11449：无监听
11441：0.0.0.0:11441，原生产 cc-proxy PID 3603271
```

本次未修改源码、生产配置、systemd 或生产服务。
