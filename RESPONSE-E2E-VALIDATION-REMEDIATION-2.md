# Responses E2E 验收补充记录（增量修复第 2 轮）

- 验收日期：2026-08-06
- 临时旁路：`127.0.0.1:11449 -> http://clawbot:11434`
- 生产边界：未触碰 `11441`；未修改生产进程、配置或 systemd。
- 凭证：仅使用脱敏占位 Authorization；本记录不含 token。
- 结论：**CONDITIONAL / P1 仍未关闭**。本轮确认 Responses 转换代码可正确产生真实 continuation wire item，且可观察真实 usage；但上游 continuation 仍出现 timeout，稳定 prefix 连续请求仍报告 `cache_creation_input_tokens` 而不是 `cache_read_input_tokens`。现有证据不能把问题归因于本地转换代码；上游能力/配置或上游状态仍是阻塞项。

## 1. 前置检查与临时实例

实际检查结果：

```text
cargo test --locked -> exit 0；77 passed, 0 failed
127.0.0.1:11441/health -> HTTP 200；body={"service":"cc-proxy","status":"ok"}
127.0.0.1:11449/health（启动前） -> 无监听
clawbot:11434/v1/models -> HTTP 200
clawbot:11434/v1/responses（GET 探测） -> HTTP 404（GET 不作为 Responses 能力结论；实际 POST 由临时代理执行）
```

临时实例启动参数：

```text
LISTEN_ADDR=127.0.0.1:11449
ESWITCH_URL=http://clawbot:11434
MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml
DEEPSEEK_API_KEY=not-needed
RUST_LOG=info
target/debug/cc-proxy
```

启动日志确认：

```text
Loaded 5 model profiles, 4 providers
Listening on: 127.0.0.1:11449
eswitch health check: OK
Server ready on 127.0.0.1:11449
```

## 2. 真实 function_call_output continuation

本轮通过临时 `11449` 的 Anthropic `/v1/messages` 入口执行 3 次真实 tool 链路。每次首轮响应均为真实 `stop_reason=tool_use`，返回真实 `tool_use.id`；continuation 使用该 ID 生成 wire item：

```json
{"type":"function_call_output","call_id":"<真实 tool_use.id>","output":"Sunny, 25C"}
```

脱敏结果：

```text
ROUND 1 initial: HTTP 200, 2.50s, stop_reason=tool_use, tool id present
ROUND 1 continuation: HTTP 200, 1.57s, stop_reason=end_turn, text content
ROUND 2 initial: HTTP 200, 1.84s, stop_reason=tool_use, tool id present
ROUND 2 continuation: client TimeoutError at 70.01s, no HTTP response
ROUND 3 initial: HTTP 200, 2.09s, stop_reason=tool_use, tool id present
ROUND 3 continuation: HTTP 200, 1.43s, stop_reason=end_turn, text content
```

判定：单次真实 continuation 成功 2/3；重复稳定性失败，不能按“连续两轮稳定”关闭 P1。timeout 只计失败证据，不计 HTTP 200、不计 cache 命中。

## 3. 同一稳定 prefix 的 cache 观察

使用固定 system prefix（约 2,916 个 prefix input tokens；仅改变 user probe 编号）连续发送 4 次 POST。临时代理日志对请求记录相同 Responses prefix fingerprint：

```text
prefix_fingerprint=121c08fdb4c7e895
```

真实 usage：

```text
CACHE request 1: HTTP 200, 2.84s,
  input_tokens=2919, output_tokens=32,
  cache_read_input_tokens=0, cache_creation_input_tokens=2916
CACHE request 2: client TimeoutError at 70.07s, no HTTP response
CACHE request 3: HTTP 200, 1.62s,
  input_tokens=2919, output_tokens=32,
  cache_read_input_tokens=0, cache_creation_input_tokens=2916
CACHE request 4: HTTP 200, 1.76s,
  input_tokens=2919, output_tokens=32,
  cache_read_input_tokens=0, cache_creation_input_tokens=2916
```

判定：可完成请求的 `cache_read_input_tokens / stable prefix tokens = 0 / 2916 = 0%`；`cache_creation_input_tokens=2916` 是 cache 写入/创建字段，不能当作 read 命中。timeout 请求没有 usage，不能计入命中或未命中。

## 4. 代码与归因结论

本轮未修改源代码。现有代码检查与测试证据：

- `src/responses/request.rs` 将 Anthropic `tool_use` 转换为 `function_call`，将真实 `tool_result.tool_use_id` 转换为 `function_call_output.call_id`；测试 `tool_result_is_function_call_output_and_arguments_are_json` 通过。
- 请求日志保留 `static_prefix_hash`、`history_prefix_hash`、`wire_input_hash` 和 item 类型；没有用本地 fingerprint 伪造上游 cache read。
- `src/responses/response.rs` 分开记录 `cached_tokens` -> `cache_read_input_tokens` 与 `cache_write_tokens` -> `cache_creation_input_tokens`；没有把 creation 当 read。
- `cargo test --locked` 实际结果为 77/77 通过。

因此当前阻塞不是已证实的本地 Responses item 转换错误。剩余 P1 需要上游 provider/cache 实际支持并稳定返回 cached/read usage，且需要上游在重复 `function_call_output` continuation 上稳定完成响应。可能阻塞项为上游 cache 策略/TTL/路由条件、上游 Responses POST 能力或服务端负载/超时；本轮没有上游内部控制面或 provider cache 配置权限，无法在本仓库内关闭这些条件。

## 5. 清理与生产健康

临时实例已发送 SIGTERM 并正常退出。最终复核：

```text
127.0.0.1:11449/health -> 无监听（curl connection refused）
127.0.0.1:11441/health -> HTTP 200
11441 body -> {"service":"cc-proxy","status":"ok"}
```

最终判定：**CONDITIONAL，不能批准生产部署；P1 仍需上游/provider 侧排查 cache-read 条件及 continuation timeout/502，并在同一临时旁路上重新验收。**
