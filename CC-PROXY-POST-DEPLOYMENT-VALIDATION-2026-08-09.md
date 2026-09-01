# cc-proxy 部署后测试与投产验收方案

**文档日期**：2026-08-09  
**适用版本**：`f6425e8`（`feat(responses): add proxy stream observability`）  
**仓库**：`https://github.com/zwczwczwc/cc-proxy`  
**工作目录**：`/root/projects/codewhale-proxy/source`

## 0. 当前状态与重要边界

### 0.1 代码是否已 push

当前代码**尚未 push 到 GitHub**。

现场核对结果：

```text
本地 HEAD：       f6425e85c7b04872f29a631809a729ceb0168ce9
origin/master：   1b57681e58606a67025c6540a50d577119767e90
本地状态：        master ahead 1
```

本地 commit：

```text
f6425e8 feat(responses): add proxy stream observability
```

本次 commit 只包含 5 个 Rust 源码文件；工作树中已有的未跟踪 Markdown 和 `tools/` 未包含在该 commit 中。

### 0.2 当前生产状态

```text
生产服务：        cc-proxy.service active
生产 MainPID：    3340
生产监听：        0.0.0.0:11441
生产二进制：      /usr/local/bin/cc-proxy
生产 SHA-256：    8c43658d854e70c90d11328b5edecd4bc420ddc0ffc208214a69f4babe766884
```

候选 artifact：

```text
/root/projects/codewhale-proxy/source/target/release/cc-proxy
候选 SHA-256：    53aba3bcc29a1bbb6c93b0d005863317d35ec23d1effa783463c0079e4f8dc50
```

**当前生产二进制尚未被替换。** 本文是部署后的测试方案，不代表已经完成生产部署。

## 1. 本次改造的测试目标

本次改造只增加 Responses 可观测性，不应改变既有协议行为：

- Responses 内部 request correlation ID；
- upstream headers/status；
- headers latency；
- streaming first-byte latency；
- Responses terminal event；
- upstream read error；
- EOF；
- EOF without terminal event；
- 非流式 Responses 完成耗时。

`request_id` 使用 `#[serde(skip)]`，不得进入：

- upstream JSON；
- Responses input/history；
- `static_prefix_hash`；
- `history_prefix_hash`；
- `wire_input_hash`；
- tools；
- tool continuation 的 `call_id`。

## 2. 部署前置门

只有以下条件全部满足，才允许进入生产替换：

```text
[ ] 明确维护窗口，并确认当前没有需要保留的生产长请求
[ ] 本地 commit SHA 已记录
[ ] 候选 artifact SHA 已记录
[ ] cargo check/test/fmt/clippy/build 全部通过
[ ] 生产二进制和配置已完成备份
[ ] 生产当前 MainPID、ExecStart、11441 listener 已记录
[ ] 回滚命令已准备
[ ] 已确认不会使用 11441 作为候选测试端口
```

推荐只读检查：

```bash
cd /root/projects/codewhale-proxy/source

git rev-parse HEAD
git status --short --branch
sha256sum target/release/cc-proxy /usr/local/bin/cc-proxy
systemctl show cc-proxy.service \
  -p MainPID -p ExecStart -p ActiveEnterTimestamp -p NRestarts --no-pager
ss -ltnp | grep -E ':11441\\b|:11449\\b' || true
```

## 3. 生产部署步骤

> 以下步骤只有在用户明确通知部署后执行。当前未执行。

### 3.1 创建备份

使用时间戳目录，例如：

```bash
TS=$(date +%Y%m%d-%H%M%S)
BACKUP=/data/backups/cc-proxy/$TS
mkdir -p "$BACKUP"
cp -p /usr/local/bin/cc-proxy "$BACKUP/cc-proxy.before"
cp -p /etc/cc-proxy/config.toml "$BACKUP/config.toml.before"
systemctl cat cc-proxy.service > "$BACKUP/cc-proxy.service.before"
sha256sum "$BACKUP/cc-proxy.before" "$BACKUP/config.toml.before"
```

不要把 token、Authorization 值或完整环境变量写入测试报告。

### 3.2 停止、替换、启动

运行中的 ELF 不直接覆盖，避免 `Text file busy`：

```bash
systemctl stop cc-proxy.service
cp /root/projects/codewhale-proxy/source/target/release/cc-proxy /usr/local/bin/cc-proxy
chmod 0755 /usr/local/bin/cc-proxy
systemctl daemon-reload
systemctl start cc-proxy.service
```

### 3.3 启动后身份确认

```bash
systemctl is-active cc-proxy.service
systemctl show cc-proxy.service \
  -p MainPID -p ExecStart -p ActiveEnterTimestamp -p NRestarts --no-pager
sha256sum /usr/local/bin/cc-proxy
ss -ltnp | grep -E ':11441\\b'
```

必须确认：

```text
service active
MainPID 是新进程
ExecStart=/usr/local/bin/cc-proxy
生产 binary SHA == 候选 SHA
11441 监听存在
NRestarts 未异常增加
```

## 4. 必测业务回归

所有请求都必须区分：

```text
HTTP 200
HTTP 400/schema error
HTTP 401/auth error
HTTP 502/504/upstream error
timeout
EOF without terminal
cache_read
cache_creation
cache_miss
```

不要把 `cache_creation` 统计为 cache hit，也不要把 401、502、timeout 统计为 cache miss。

### 4.1 Health

```bash
curl --noproxy '*' -fsS --max-time 5 \
  http://127.0.0.1:11441/health
```

health 只能证明服务存活，不能替代下列业务测试。

### 4.2 既有 Chat 路径回归

目标：确认 DeepSeek/GLM/Kimi 等非 Responses profile 仍走原 Chat 路径。

至少执行：

```text
DeepSeek Chat：HTTP 200，响应正文和 usage 可解析
GLM Chat：HTTP 200，响应正文和 usage 可解析
Kimi Chat：HTTP 200，响应正文和 usage 可解析（如当前配置启用）
```

同时检查日志仍使用：

```text
OpenAI request built
KV cache stats
```

不能出现 Chat 请求被静默改走 Responses，也不能出现 Responses 失败后隐式 fallback 到 Chat。

### 4.3 Responses 非流式文本

请求入口仍是 Anthropic Messages：

```bash
curl --noproxy '*' -sS --max-time 120 \
  -w '\\nHTTP:%{http_code}\\n' \
  http://127.0.0.1:11441/v1/messages \
  -H 'Authorization: Bearer not-needed' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "claude-sonnet-4-6",
    "max_tokens": 128,
    "thinking": {"type": "enabled", "budget_tokens": 256},
    "messages": [
      {"role": "user", "content": "Reply with exactly OK."}
    ],
    "stream": false
  }'
```

验收：

```text
HTTP 200
返回 Anthropic message JSON
stop_reason 可解析
正文可解析
journal 出现 Responses response headers
journal 出现 Responses non-stream completed
journal 出现 Responses cache stats
```

### 4.4 Responses streaming 文本

```bash
curl --noproxy '*' -sS --max-time 120 -N \
  -w '\\nHTTP:%{http_code}\\n' \
  http://127.0.0.1:11441/v1/messages \
  -H 'Authorization: Bearer not-needed' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "claude-sonnet-4-6",
    "max_tokens": 128,
    "thinking": {"type": "enabled", "budget_tokens": 256},
    "messages": [
      {"role": "user", "content": "Reply with exactly OK."}
    ],
    "stream": true
  }'
```

验收事件：

```text
message_start
content_block_start
content_block_delta
content_block_stop
message_delta
message_stop
```

日志验收：

```text
Responses request telemetry request_id=<内部ID>
Responses stream response headers request_id=<同一内部ID>
Responses stream first byte request_id=<同一内部ID>
Responses stream terminal event request_id=<同一内部ID> terminal_event=response.completed
Responses cache stats status="completed"
```

### 4.5 Tool continuation：必须使用 fresh call ID

这是本次投产的关键门，至少做 **2 条独立链路**。

第一轮必须强制模型调用一个测试工具：

```json
{
  "model": "claude-sonnet-4-6",
  "max_tokens": 256,
  "thinking": {"type": "enabled", "budget_tokens": 256},
  "tools": [
    {
      "name": "probe_tool",
      "description": "Return the supplied value.",
      "input_schema": {
        "type": "object",
        "properties": {"value": {"type": "string"}},
        "required": ["value"]
      }
    }
  ],
  "tool_choice": {"type": "tool", "name": "probe_tool"},
  "messages": [
    {"role": "user", "content": "Call probe_tool with value test."}
  ],
  "stream": false
}
```

验收第一轮：

```text
HTTP 200
stop_reason=tool_use
存在真实 tool_use.id
存在真实 tool_use.name
存在可解析的 tool_use.input
```

然后把第一轮返回的**完整 assistant content**原样放入下一轮，并使用本轮刚返回的真实 ID：

```json
{
  "model": "claude-sonnet-4-6",
  "max_tokens": 256,
  "messages": [
    {"role": "user", "content": "Call probe_tool with value test."},
    {
      "role": "assistant",
      "content": "<第一轮返回的完整 content 数组>"
    },
    {
      "role": "user",
      "content": [
        {
          "type": "tool_result",
          "tool_use_id": "<第一轮真实 tool_use.id>",
          "content": "probe result: test"
        }
      ]
    }
  ],
  "tools": ["<同一工具定义>"],
  "stream": false
}
```

验收：

```text
continuation HTTP 200
最终 stop_reason=end_turn
最终正文可解析
cc-proxy Responses input 日志包含：
  role:user
  function_call
  function_call_output
```

禁止：

- 使用 placeholder call ID；
- 重复使用上一条已经消费的 call ID；
- 只验证首轮 HTTP 200 而不验证续接；
- 把“模型没有返回 tool_use”当作 continuation 成功。

### 4.6 Streaming tool continuation

至少额外验证一条 streaming tool-call 链路：

```text
Anthropic stream 中出现 tool_use block
input_json_delta 可拼接为合法 JSON
message_delta.stop_reason=tool_use
使用真实 tool_use.id 发送 tool_result
下一轮最终出现 message_stop
```

## 5. 长前缀 cache 验收

使用固定的完整 `instructions + input + tools`，建议稳定前缀至少 5.8K tokens，连续发送至少 4 次有效 HTTP 200。

记录每次：

```text
input_tokens
cache_read_input_tokens
cache_creation_input_tokens
cache_miss_input_tokens
hit_rate
HTTP status
history_prefix_hash
wire_input_hash
```

判定：

- 第一次通常是 cache creation，不计为 hit；
- 后续请求必须单独看 `cache_read_input_tokens`；
- 固定 wire 的后续请求能够命中，才说明候选部署后的 cache wire 没有被改坏；
- 多轮增长历史不要求 `cache_read / 当前总 input` 达到 90%，应比较公共稳定前缀。

## 6. 观测性验收

### 6.1 正常 streaming

同一个 `request_id` 必须能串起：

```text
Responses request telemetry
→ Responses stream response headers
→ Responses stream first byte
→ Responses cache stats
→ Responses stream terminal event
```

### 6.2 非 2xx upstream

在候选环境或独立故障测试环境模拟 upstream 4xx/5xx 后，应看到：

```text
Responses stream response headers upstream_http_status!=200
Responses stream upstream request failed
```

且不会记录成功 `message_stop`。

### 6.3 EOF without terminal

在测试环境让 upstream 在发送部分 SSE 后断开，验收：

```text
Anthropic error/stream_error
没有成功 message_stop
没有成功 end_turn
日志：Responses stream EOF without terminal event
```

该项不建议在生产 11441 上直接做，优先用单元测试或隔离候选进程验证。

### 6.4 下游客户端取消

在测试环境请求收到首字节后主动断开客户端连接，检查：

```text
候选进程无 goroutine/连接泄漏
upstream stream 最终可结束
服务仍可接受下一条请求
```

如尚未增加下游 disconnect 日志，必须在报告中明确“行为已检查、专用日志未覆盖”，不能假设已观测。

## 7. 部署后日志查询

### 7.1 新版本启动确认

```bash
journalctl -u cc-proxy.service --since '部署时间' --no-pager -o short-iso
```

必须确认：

```text
Loaded model config from /etc/cc-proxy/config.toml
Loaded model profiles/providers
Server ready on 0.0.0.0:11441
```

### 7.2 Responses 观测日志

```bash
journalctl -u cc-proxy.service --since '部署时间' --no-pager -o cat \
  | python3 -c '
import sys
for line in sys.stdin:
    if any(x in line for x in (
        "Responses request telemetry",
        "Responses response headers",
        "Responses stream response headers",
        "Responses stream first byte",
        "Responses stream terminal event",
        "Responses stream upstream read error",
        "Responses stream EOF",
        "Responses cache stats",
    )):
        print(line, end="")
'
```

不要只看到 `request built` 就判定请求成功；必须同时看到真实 terminal 或明确错误。

## 8. Go / No-Go 判定

### GO

只有以下全部通过才允许报告 `GO`：

```text
[ ] 候选 binary SHA 与部署 binary SHA 一致
[ ] service active，MainPID 是新进程，11441 listener 正常
[ ] Chat 回归通过
[ ] Responses 非流式通过
[ ] Responses streaming 完整到 message_stop
[ ] 至少 2 条独立 fresh-ID tool continuation 通过
[ ] streaming tool continuation 通过
[ ] 长前缀 cache 按规则完成预热与命中统计
[ ] request_id → headers → first byte → terminal 日志链完整
[ ] 无 502/504/timeout/EOF 未分类错误
[ ] 11441 真实业务回归完成
```

### CONDITIONAL

以下情况只能报告 `CONDITIONAL`：

- 只有 health 通过；
- 只有文本 streaming 通过；
- 没有真实 tool continuation；
- 没有验证非流式路径；
- 只使用旧二进制或旧报告；
- 只在 11449 验证而没有部署后的 11441 回归；
- 只看到 cache stats，但没有关联 request_id 和 terminal event。

### BLOCKED / 回滚

任一关键业务门失败时：

```bash
systemctl stop cc-proxy.service
cp "$BACKUP/cc-proxy.before" /usr/local/bin/cc-proxy
cp "$BACKUP/config.toml.before" /etc/cc-proxy/config.toml
chmod 0755 /usr/local/bin/cc-proxy
systemctl daemon-reload
systemctl start cc-proxy.service
systemctl is-active cc-proxy.service
curl --noproxy '*' -fsS --max-time 5 http://127.0.0.1:11441/health
```

回滚后必须记录：

```text
回滚 binary SHA
回滚 config SHA
服务 MainPID
11441 health
11441 listener
11449 是否无监听
失败的具体测试门
```

## 9. 当前结论

当前 commit 可以继续保留并用于后续部署，但在用户明确通知前：

```text
不 push GitHub
不替换 /usr/local/bin/cc-proxy
不重启 cc-proxy.service
不修改 /etc/cc-proxy/config.toml
```

当前文档只定义部署后的测试工作，不代表这些生产测试已经执行。