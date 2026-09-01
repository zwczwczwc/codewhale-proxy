# gpt-5.6-luna Responses Cache 现场问题交接文档

> 用途：新会话启动后，先阅读本文档，再直接进入 cache/prefix 根因排查。本文档记录的是 2026-08-06 GPU 节点的 live 证据，不把旧结论当作事实。
>
> 安全：本文档不包含 API key、Authorization 值、Feishu secret、prompt 原文或完整工具 schema。

## 1. 当前问题

用户通过本机 `cc-connect` 向 Claude Code 发送了一条任务消息。该消息在 Claude Code 内部触发了多个模型/工具循环，而不是只产生一次上游调用。

本次 Claude Code 的 gpt-5.6-luna Responses 请求出现持续的：

```text
cache_read_input_tokens=0
cache_creation_input_tokens≈input_tokens
cache_miss_input_tokens=3
hit_rate_percent=0.0
```

用户明确指出：不能把“模型 cache 容量约 133K”作为根因。用户在 clawbot 节点通过 Hermes 使用 gpt-5.6-luna 时，观察到过 500K+ 大上下文请求也能使用 cache。因此新会话不得再以固定容量限制作为默认解释，必须用 live A/B 和 wire/prefix 证据确认根因。

目标不是立即修改生产，而是先确定以下哪一层有问题：

```text
Claude Code 请求重放/并发
→ cc-proxy Anthropic→Responses 历史序列化
→ cc-proxy 与上游的连接复用/请求关联
→ eswitch/LB 路由或 cache key
→ 上游 Responses cache
```

## 2. 当前生产现场

采集时间：2026-08-06 18:42 CST 左右。

```text
cc-proxy.service：active
cc-proxy MainPID：2164518
cc-proxy 启动：2026-08-06 16:38:45 CST
cc-connect.service：active
cc-connect MainPID：2031597
cc-connect 启动：2026-08-06 14:45:49 CST
生产监听：0.0.0.0:11441
临时旁路：127.0.0.1:11449 无监听
health：HTTP 200
上游配置：http://clawbot:11434
```

当前生产二进制：

```text
/root/projects/codewhale-proxy/source/target/release/cc-proxy
/usr/local/bin/cc-proxy
SHA-256：4458f16a7fee190cb9652e7732d718fcfdaa4c1b4831cd446161762342e1ed92
```

生产部署备份：

```text
/var/backups/cc-proxy/monitor-deploy-20260806-163845
```

当前源码分支：

```text
/root/projects/codewhale-proxy/source/
feat/gpt-responses-transport
```

工作树有大量未提交的 Responses 改造、配置和文档改动；不要执行 reset/clean/checkout 覆盖现有工作树。

## 3. 本次 Claude Code 任务证据

cc-connect 在本地时间 `17:52:47` 收到一条用户消息：

```text
message received
processing message
```

截至实时监控结束（约 18:07）没有看到对应的 `turn complete`。Claude Code 进程仍在运行，命令为：

```text
claude --output-format stream-json
  --input-format stream-json
  --permission-prompt-tool stdio
  --replay-user-messages
  --verbose
  --permission-mode bypassPermissions
  --resume 1dbad6e9-bf50-43fe-b68c-420b764a622a
  --append-system-prompt-file /home/claude/.cc-connect/agent-prompts/cc-connect-system.md
```

本次任务明显包含大量工具/子 Agent 调用；此前同一会话的 cc-connect 记录出现过 `tools=188`。因此“用户只发一条消息”不等于“只调用一次 gpt”。

## 4. 实时监控结果

实时监控器执行：

```text
journalctl -u cc-proxy.service --since now -f -o short-iso --no-pager
```

监控窗口约 10 分钟，统计：

```text
Responses request built：12 次
Responses cache stats：11 次
DeepSeek Chat request built：67 次
error/failed/timeout/502/503/504/panic：0 次
```

随后在 18:05～18:07 又观察到新的 Responses 请求和 stats。以最终已抓取日志为准，不要把 12/11 当作整个会话的最终总数；新会话应重新按明确时间窗口统计。

典型实时数据：

```text
17:55:25  input=324192  read=0  creation=324189  miss=3  rate=0.0%
17:56:01  input=324390  read=0  creation=324387  miss=3  rate=0.0%
17:56:23  input=327741  read=0  creation=327738  miss=3  rate=0.0%
17:56:53  input=327853  read=0  creation=327850  miss=3  rate=0.0%
18:00:36  input=335650  read=0  creation=335647  miss=3  rate=0.0%
18:01:06  input=335689  read=0  creation=335686  miss=3  rate=0.0%
18:01:52  input=352299  read=0  creation=352296  miss=3  rate=0.0%
18:02:34  input=352338  read=0  creation=352335  miss=3  rate=0.0%
18:03:00  input=352377  read=0  creation=352374  miss=3  rate=0.0%
18:03:22  input=352423  read=0  creation=352420  miss=3  rate=0.0%
18:04:48  input=352462  read=0  creation=352459  miss=3  rate=0.0%
18:06:48  input=352501  read=0  creation=352498  miss=3  rate=0.0%
18:07:49  后续仍有 Responses cache stats，需新会话重新解析完整窗口
```

每条 usage 都满足：

```text
cache_read + cache_creation + cache_miss = input_tokens
```

例如：

```text
0 + 352498 + 3 = 352501
```

因此这是上游返回的实际 cache usage，不是 cc-proxy 统计字段计算错误。

## 5. 调用次数与请求重放现象

在 17:52:40～18:07:30 的抽取窗口中，已确认的 Responses 构建摘要包括：

```text
17:52:48  input_item_count=225  history_prefix_hash=dbd91ec11954c416  wire=265d6b59b73ee80b
17:53:02  input_item_count=228  history_prefix_hash=1c0ddf67664cff10  wire=123886f313d78bba
18:04:49  input_item_count=299  history_prefix_hash=2285b28b9af86507  wire=44693bbf8dfc4e17
18:05:50  input_item_count=299  history_prefix_hash=2285b28b9af86507  wire=44693bbf8dfc4e17
18:06:49  input_item_count=301  history_prefix_hash=b4661a0eb9354ae9  wire=6cfb0075691f77a4
```

重要异常：完全相同的请求摘要出现两次：

```text
input_item_count=299
history_prefix_hash=2285b28b9af86507
wire_input_hash=44693bbf8dfc4e17
```

时间相隔约 61 秒：

```text
18:04:49
18:05:50
```

这需要优先调查：

1. Claude Code/cc-connect 是否发生重试或重复发送；
2. 前一个 stream 是否尚未完成，后一个相同请求是否已进入；
3. 请求是否在 cc-proxy 内部被重复处理；
4. 上游是否因未及时完成导致客户端重发；
5. 现有日志缺少 request_id，无法把 `request built` 与 terminal usage 一一关联。

不能直接断言这是重复调用根因，但这是当前最有价值的 live 证据。

## 6. hash 现状与限制

大多数 Claude Code Responses 请求的：

```text
static_prefix_hash=5fed12e3bac3d4f1
```

这说明静态部分在这些请求之间保持一致。当前 `src/responses/request.rs` 中，static hash 覆盖：

```json
{
  "model": ...,
  "instructions": ...,
  "tools": ...,
  "tool_choice": ...,
  "reasoning": ...
}
```

但 static hash **不覆盖完整 input/history**，因此不能用它证明历史前缀字节稳定。

当前 history hash 是：

```rust
canonical_hash(Value::Array(input[..history_item_count].to_vec()))
```

它代表“本次完整历史数组”的 hash，不是跨请求的“公共前缀 hash”。只要历史追加一轮，完整 history hash 就会变化。因此：

```text
history_prefix_hash 变化 ≠ 已有公共前缀被破坏
```

新会话必须增加/计算真正的跨轮公共前缀比较：

```text
上一请求 history wire bytes
与下一请求 input 的前 N 个 item/字节
逐字节或 canonical bytes 比较
```

禁止只看当前 `history_prefix_hash` 是否相同就下结论。

## 7. 已有对照实验：基础 Responses cache 并非完全失效

在同一个生产 cc-proxy `11441` 上做过受控多轮测试，稳定前缀约 15K tokens：

```text
proxy round 1: input=15415, read=0, creation=15412
proxy round 2: input=15432, read=15412, creation=17
proxy round 3: input=15449, read=15429, creation=17
```

约 99.87% 的稳定前缀能够命中。

直接访问 `http://clawbot:11434/v1/responses` 的对应多轮测试也能命中：

```text
direct round 1: input=15414, read=0, creation=15411
direct round 2: input=15430, read=0, creation=15427
direct round 3: input=15446, read=15427, creation=16
```

这两个事实共同说明：

```text
上游 gpt-5.6-luna Responses cache 能工作；
cc-proxy 的基础 Responses 序列化/缓存路径不是所有场景都失效。
```

但这不能证明 300K+、包含大量 function_call/function_call_output 的 Claude Code wire 正确；需要做同语义 A/B。

## 8. 当前最重要的未决假设

### 假设 A：Claude Code/cc-connect 请求重叠或重放

证据：同一 `wire_input_hash` 在约 61 秒后重复构建；单次大请求耗时几十秒。

验证方法：为每次请求加入 request_id、start/end、stream terminal、duration，并在 cc-connect/Claude JSONL 中关联 message/tool turn。若相同 wire 请求重叠，先修请求生命周期/重试，不要改 cache 算法。

### 假设 B：历史公共前缀在 Responses wire 中不稳定

证据：static hash 稳定，但 history/wire hash 随 tool history 变化；现有 hash 不足以证明公共前缀。

验证方法：在转换后保留脱敏的 canonical input bytes/digest 分段（只保存 hash，不保存 prompt），对相邻请求计算：

```text
common_item_count
common_byte_length
common_prefix_hash
first_divergent_item_index
```

若下一请求没有完整复用上一请求的历史前缀，检查：

```text
assistant output_text 序列化
function_call/function_call_output 顺序
tool result 文本
reasoning block 丢弃/重放
synthetic volatile tail
tools/instructions 排序和规范化
```

### 假设 C：上游按连接/LB 分区，Hermes 与 cc-proxy 走到不同 cache 分区

证据：用户的 Hermes 500K+ 经验与 cc-proxy 300K+ 结果冲突；当前 cc-proxy 有一条到 `100.64.0.1:11434` 的 keep-alive 连接。

验证方法：同一 wire 做：

```text
A：直接 clawbot:11434，复用连接
B：直接 clawbot:11434，每次新连接
C：cc-proxy 11441，当前连接池
D：临时 11449 旁路，每次新连接（测试后清理）
```

每组至少 3 个 warmup/有效 HTTP 200 样本，比较 usage、hash 和耗时。不能用短请求或不同语义 prompt 代替。

### 假设 D：Claude Code 与 Hermes 的 wire 结构不同，导致 cache key 不同

必须比较而不是推测：

```text
instructions
input item types/order
assistant output_text
function_call
function_call_output
reasoning
tools/tool_choice
synthetic tail
stream/non-stream
```

用户 Hermes 的 500K+ 经验是重要反证，不能再用“模型容量限制”替代 wire A/B。

## 9. 新会话第一步：只读排查命令

进入：

```text
cd /root/projects/codewhale-proxy/source
```

先读：

```text
RESPONSES-CACHE-LIVE-DIAGNOSTIC-HANDOFF-2026-08-06.md
RESPONSES-CACHE-MONITORING-VALIDATION.md
src/responses/request.rs
src/responses/response.rs
src/responses/stream.rs
src/responses/types.rs
src/client.rs
```

核对 live：

```bash
systemctl status cc-proxy.service --no-pager
systemctl status cc-connect.service --no-pager
systemctl show cc-proxy.service -p MainPID -p ExecStart -p Environment --no-pager
ss -tnp | grep -E '11441|11434'
journalctl -u cc-proxy.service --since '10 min ago' --no-pager
```

统计一次明确用户任务窗口：

```bash
journalctl -u cc-proxy.service \
  --since '<START>' --until '<END>' --no-pager -o short-iso \
  | grep -E 'Responses request built|Responses cache stats|Responses stream|error|timeout|502|503|504'
```

注意日志中的 `input_item_types` 可能很长，输出时必须脱敏/截断；不要打印 prompt、工具 schema 或凭证。

## 10. 推荐排查顺序

### 阶段 1：补齐请求关联观测（不改变 wire）

先写 RED 测试，再最小实现：

```text
request_id
request_started_at
request_duration_ms
wire_input_hash
history_prefix_hash
common-prefix diagnostic hashes（只保存 hash）
terminal_event_seen
stream_error
```

要求：同一请求的 `request built`、upstream response/stream terminal、cache stats 使用同一 request_id。不得记录原文。

### 阶段 2：验证重复/重叠请求

基于 request_id 和时间区间判断：

```text
同 wire 是否同时在飞
同 wire 是否第二次开始时第一次尚未 terminal
是否是客户端重试
是否有上游 terminal 但客户端未收到
```

### 阶段 3：验证跨轮公共前缀

对同一 Claude 会话的相邻请求，比较真实转换后的 canonical item bytes：

```text
第 N 请求完整 history
第 N+1 请求 input 的前 N history items
```

必须输出：

```text
common_item_count
common_prefix_hash
first_divergent_item
```

### 阶段 4：做同语义 direct-vs-proxy A/B

禁止直接修改生产配置。优先：

```text
direct clawbot:11434
current production 11441
```

只有必要时，使用 `127.0.0.1:11449` 临时旁路；结束后确认无监听。

### 阶段 5：再判断上游/LB/连接池

只有当 semantic wire bytes 已证明一致，才比较：

```text
keep-alive vs 新连接
不同连接的 cache usage
响应耗时
上游 status/error
```

## 11. 禁止的错误结论

不要再次直接说：

```text
gpt-5.6-luna cache 容量约 133K，所以 300K 请求必然无法命中
```

理由：用户已提供同环境 Hermes 500K+ cache 命中经验，且当前没有完成权威/同路径验证。

也不要说：

```text
static_prefix_hash 相同，所以完整历史前缀一定相同
```

当前 static hash 明确不包含完整 input。

不要把：

```text
cache_creation
```

当作：

```text
cache_read
```

也不要把短请求的 `cache_read=0` 当作 cache 系统失败。

## 12. 当前状态总结

```text
Responses cache 日志改造：已部署并生效
gpt-5.6-luna Responses 路由：正常
Claude Code 单条用户任务内部调用次数：多次，至少十余次 Responses 构建/完成
本次长任务 cache_read：持续为 0
本次长任务 cache_creation：接近完整 input
static_prefix_hash：稳定为 5fed12e3bac3d4f1
history/wire hash：随历史增长变化
同一 wire hash：出现过重复构建
错误：当前窗口未见 400/401/502/503/504/timeout/panic
cc-proxy：active，health 200
cc-connect：active
11449：无监听
```

当前最合理的工作假设不是“模型 cache 容量限制”，而是：

```text
请求生命周期/重放
或
cc-proxy 转换后跨轮公共前缀不稳定
或
Hermes 与 cc-proxy 的连接/LB/cache 分区不同
```

新会话应从 request_id + common-prefix digest + direct/proxy A/B 开始，不要先改生产模型路由、effort、thinking 或 cache 算法。
