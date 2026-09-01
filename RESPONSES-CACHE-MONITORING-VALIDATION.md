# Responses Cache Monitoring Validation

日期：2026-08-06
分支：`feat/gpt-responses-transport`

## 实施结果

- Responses 非流式和流式都输出 `Responses cache stats`。
- 流式只在当前 `response.completed` 或 `response.incomplete` 事件携带最终 usage 时记录一次。
- 重复 terminal 事件不会重复记录。
- 统计字段包括：
  - `input_tokens`
  - `cache_read_input_tokens`
  - `cache_creation_input_tokens`
  - `cache_miss_input_tokens`
  - `hit_rate_percent`
  - `status`
  - `upstream_http_status`
- cache creation/write 不会被当成 cache read。
- 无 usage、JSON parse error、failed/error、非 200、请求错误不会伪造 cache miss。
- Chat 原有 `KV cache stats` 路径未修改。
- 日志不包含 prompt、Authorization、API key、完整工具 schema 或原始响应。

## TDD 与质量门

```text
RED：测试先于 cache_stats_from_usage 实现执行并失败
GREEN：86 passed; 0 failed
cargo fmt --all -- --check：PASS
cargo clippy --all-targets --all-features -- -D warnings：PASS
git diff --check：PASS
cargo build --release --locked：PASS
```

## 生产部署

本次由 Main Agent 直接接管生产部署，备份与替换：

```text
backup directory: /var/backups/cc-proxy/monitor-deploy-20260806-163845
old binary SHA-256: 9278544d800086550ba7cf2b9a454692f4a2b71028d3c9d22001a36b3be1417c
new binary SHA-256: 4458f16a7fee190cb9652e7732d718fcfdaa4c1b4831cd446161762342e1ed92
```

部署顺序：

```text
systemctl stop cc-proxy.service
replace /usr/local/bin/cc-proxy
systemctl start cc-proxy.service
```

最终状态：

```text
cc-proxy.service: active/running
11441 health: HTTP 200
11441 listener: present
11449 listener: absent
```

## 真实生产验证

测试入口：生产 `11441/v1/messages`；未使用临时 `11449`。

### Responses 非流式

```text
HTTP 200
model: gpt-5.6-luna
stop_reason: end_turn
```

日志：

```text
Responses cache stats ... input_tokens=12 cache_read_input_tokens=0 cache_creation_input_tokens=0 cache_miss_input_tokens=12 hit_rate_percent=0.0
```

### Chat 回归

```text
HTTP 200
model: claude-sonnet-4-5
```

原有 Chat 路径仍可用。

### Responses 流式

```text
HTTP 200
Content-Type: text/event-stream
```

生产日志出现流式终端 usage 对应的：

```text
Responses cache stats ... input_tokens=12 cache_read_input_tokens=0 cache_creation_input_tokens=0 cache_miss_input_tokens=12 hit_rate_percent=0.0
```

### 长前缀 cache

固定前缀约 9851 input tokens，固定 tools 和 user 文本：

```text
request 1: input_tokens=9851, cache_read=0, cache_creation=9848, cache_miss=3, hit_rate=0.0%
request 2: input_tokens=9851, cache_read=9848, cache_creation=0, cache_miss=3, hit_rate=99.969546%
request 3: input_tokens=9851, cache_read=9848, cache_creation=0, cache_miss=3, hit_rate=99.969546%
request 4: input_tokens=9851, cache_read=9848, cache_creation=0, cache_miss=3, hit_rate=99.969546%
```

本次真实生产日志已直接证明 Responses 监控可观察到：

```text
cache_creation
cache_read
cache_miss
hit_rate_percent
```

并且长 prefix 后续命中率约 `99.97%`。

## 错误分类

本次监控部署和业务验证窗口内未观察到：

```text
400 / 401 / 502 / 503 / 504
client timeout
panic
```

## 结论

Responses 非流式和流式 cache 监控已完成代码实现、质量门、生产部署和真实业务验证。之后可使用：

```bash
journalctl -u cc-proxy.service | grep 'Responses cache stats'
```

检查 gpt-5.6-luna 的 cache read、cache creation、cache miss 和 hit rate。
