# Responses Cache / Continuation A/B 实际业务测试报告

- 测试时间：2026-08-06
- 工作树：`/root/projects/codewhale-proxy/source`
- 分支：`feat/gpt-responses-transport`
- A 组：直连 `http://clawbot:11434/v1/responses`
- B 组：临时 `http://127.0.0.1:11449/v1/messages` → cc-proxy → `http://clawbot:11434/v1/responses`
- 生产边界：未触碰 `11441`；临时进程结束后已清理。
- 认证：使用运行时占位 Authorization；本报告不记录 Authorization 值、token、prompt 原文或工具 schema 原文。

## 1. 执行前检查

实际检查：

```text
branch: feat/gpt-responses-transport
clawbot: 100.64.0.1
生产 11441：原有 cc-proxy 监听
临时 11449：启动前无监听
本机 11434：未作为 Responses 目标
```

直接上游最小 POST 探测：

```text
POST http://clawbot:11434/v1/responses
HTTP 200
响应 model: gpt-5.6-luna
usage.input_tokens_details 存在
```

临时 cc-proxy 启动：

```text
LISTEN_ADDR=127.0.0.1:11449
ESWITCH_URL=http://clawbot:11434
MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml
RUST_LOG=info
```

启动日志：

```text
Loaded 5 model profiles, 4 providers
Listening on: 127.0.0.1:11449
eswitch health check: OK
Server ready on 127.0.0.1:11449
```

## 2. 长前缀 cache A/B

### 测试向量

- `gpt-5.6-luna`；
- 固定 instructions/system 前缀，字符数 `172520`；
- 约 `30450` 个 prefix tokens；
- 固定 tools；
- 固定最终 user 文本；
- 重复 4 次；
- A 组使用 Responses wire；B 组使用 Anthropic `/v1/messages`，由 cc-proxy 转为同一 Responses 语义。

说明：A 和 B 的外层请求协议不同，因此外层 JSON hash 不相同；本测试同时记录固定前缀 hash 和各自 wire/semantic hash，不能把两个外层 JSON hash 当作必须相等。

### A 组：直接 Responses

固定前缀摘要：

```text
PREFIX_HASH=ed8b383f0f341b3e
DIRECT_BODY_HASH=7a9d82f12920885c
PREFIX_CHARS=172520
```

真实结果：

```text
request 1: HTTP 200, input=30453, cache_read=0,     cache_creation=30450
request 2: HTTP 200, input=30453, cache_read=0,     cache_creation=30450
request 3: HTTP 200, input=30453, cache_read=30450, cache_creation=0
request 4: HTTP 200, input=30453, cache_read=30450, cache_creation=0
```

A 组第 3/4 次读取命中率：

```text
30450 / 30453 ≈ 99.9902%
```

### B 组：cc-proxy 旁路

固定前缀摘要：

```text
PREFIX_HASH=ed8b383f0f341b3e
PROXY_SEMANTIC_HASH=2f71fe2d9f40fece
PREFIX_CHARS=172520
```

B 组的外层请求是 Anthropic `/v1/messages`，因此 `PROXY_SEMANTIC_HASH` 与 A 组完整 Responses body hash 不应直接比较；关键是 prefix 内容、模型、tools 和最终 Responses 语义保持稳定。

真实结果：

```text
request 1: HTTP 200, input=30453, cache_read=30450, cache_creation=0
request 2: HTTP 200, input=30453, cache_read=30450, cache_creation=0
request 3: HTTP 200, input=30453, cache_read=30450, cache_creation=0
request 4: HTTP 200, input=30453, cache_read=30450, cache_creation=0
```

B 组连续有效请求读取命中率：

```text
30450 / 30453 ≈ 99.9902%
```

由于 A 组先完成了两次 cache creation，B 组开始时已处于同一上游 cache 热状态；这说明 cc-proxy 旁路能够复用该 Responses cache，而不是破坏 cache prefix。

## 3. Tool continuation A/B

每次测试都重新发首轮请求并读取新的真实 tool call ID；没有重复提交已经消费的 call ID。

### A 组：直接 Responses

```text
独立链路 1：首轮 HTTP 200，function_call，真实 call_id；续接 HTTP 200，message，completed
独立链路 2：首轮 HTTP 200，function_call，真实 call_id；续接 HTTP 200，message，completed
独立链路 3：首轮 HTTP 200，function_call，真实 call_id；续接 HTTP 200，message，completed
```

### B 组：cc-proxy 旁路

```text
独立链路 1：首轮 HTTP 200，tool_use，真实 tool_use.id；续接 HTTP 200，text，end_turn
独立链路 2：首轮 HTTP 200，tool_use，真实 tool_use.id；续接 HTTP 200，text，end_turn
独立链路 3：首轮 HTTP 200，tool_use，真实 tool_use.id；续接 HTTP 200，text，end_turn
```

本次 A/B 测试中没有出现 timeout 或 502；这与此前的间歇性 timeout 记录不同，但本报告只对本次实际样本负责，不能据此宣称上游永不超时。

## 4. Streaming A/B

### A 组：直接 Responses

```text
HTTP 200
Content-Type: text/event-stream
bytes: 6082
包含 response.completed
```

直接 Responses 使用 Responses 原生 SSE data 形态，测试脚本的通用 `event:` 计数为 0，不能据此否定流；`response.completed` 已观察到。

### B 组：cc-proxy 旁路

```text
HTTP 200
Content-Type: text/event-stream
bytes: 975
events: 8
包含 message_stop
```

B 组成功将 Responses stream 转换为 Anthropic SSE。

## 5. 责任边界判断

本次结果给出以下强证据：

1. A 组直接 Responses 在固定长前缀下第 3/4 次达到约 `99.9902%` cache read；上游 cache 实际可用。
2. B 组 cc-proxy 旁路在同一固定前缀热状态下 4/4 次达到约 `99.9902%` cache read；本次没有观察到 cc-proxy 使 cache 失效。
3. A/B 三次独立真实 tool continuation 均成功；本次没有重现此前 timeout/502。
4. B 组 SSE 以 Anthropic `message_stop` 完成；A 组观察到原生 `response.completed`。
5. 因此此前“cache_read=0/timeout”不能再笼统归因于 cc-proxy，也不能仅凭此前少量失败样本断言 provider 必然阻塞。更准确的结论是：当前链路在本次长前缀热状态和独立 continuation 样本下工作；此前失败具有间歇性，可能与上游/LB 状态、缓存预热/淘汰、连接复用或当时测试向量有关，仍需持续监控。

## 6. 清理与生产检查

```text
临时 cc-proxy：已 SIGTERM
11449：测试后无监听
生产 11441：仍由原有 cc-proxy 监听
生产服务：未重启、未修改
```

## 7. 最终判定

本次 A/B 业务测试：**PASS（本次样本）**。

已实际证明：

```text
直接 Responses 长前缀 cache：约 99.9902%
cc-proxy Responses 旁路长前缀 cache：约 99.9902%
直接 Responses continuation：3/3 成功
cc-proxy continuation：3/3 成功
Responses 原生 stream：HTTP 200 + response.completed
cc-proxy Anthropic stream：HTTP 200 + message_stop
```

限制：本次测试是一个成功样本，不能把它等同于对所有未来请求的永久稳定性保证；此前间歇性 timeout/502 仍应保留为监控项。生产部署仍需独立审批和发布流程，本次没有生产部署。
