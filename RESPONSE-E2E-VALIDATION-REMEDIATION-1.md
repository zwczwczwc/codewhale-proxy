# Responses E2E 验收补充记录（增量修复第 1 轮）

- 验收日期：2026-08-05
- 临时旁路：`127.0.0.1:11449 -> http://clawbot:11434`
- 生产边界：未触碰 `11441`；未修改生产进程、配置或 systemd。
- 凭证：仅使用脱敏占位 Authorization；本记录不含 token。
- 结论：**CONDITIONAL / P1 仍未关闭**。真实 continuation 可偶发成功，但未达到两轮稳定要求；长前缀请求本次取得连续 3 个 HTTP 200，但 `cache_read_input_tokens` 仍为 0，未达到 >=90% 命中要求。

## 1. 环境与前置检查

实际检查结果：

```text
getent hosts clawbot -> 100.64.0.1 clawbot.hermes.tailnet clawbot
upstream GET/POST probe -> HTTP 200
11441 -> HTTP 200，既有 cc-proxy PID 3603271，未停止/重启
```

临时实例实际启动日志：

```text
LISTEN_ADDR=127.0.0.1:11449
ESWITCH_URL=http://clawbot:11434
MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml
Loaded 5 model profiles, 4 providers
eswitch health check: OK
Server ready on 127.0.0.1:11449
```

## 2. 真实 function-call continuation

请求入口为临时实例 Anthropic `/v1/messages`，tool 定义为 `lookup_weather`，每次首轮均由响应返回真实 `tool_use.id`，后续请求使用该 ID 作为 `tool_result.tool_use_id`。

### 2.1 第一次链路

```text
ROUND 1 initial: HTTP 200, elapsed 1.790s, stop_reason=tool_use,
  content_types=[tool_use], id=msg_71efd9b296ae4770be9f35f20dbac7af
ROUND 1 continuation: HTTP 200, elapsed 1.335s, stop_reason=end_turn,
  content_types=[text]
ROUND 1 repeated continuation: HTTP 502, elapsed 3.887s
```

说明：首个 continuation 是真实 call_id 链路且成功；重复提交同一历史/结果后返回 502，不计成功。

### 2.2 重复链路

```text
ROUND 2 initial: HTTP 200, elapsed 2.017s, stop_reason=tool_use,
  真实 tool_use.id 存在
ROUND 2 continuation: 客户端 ReadTimeout 45.041s，未收到 HTTP 响应，不计成功
ROUND 3 initial: HTTP 200, elapsed 2.532s, stop_reason=tool_use,
  真实 tool_use.id 存在
ROUND 3 continuation: HTTP 200, elapsed 1.936s, stop_reason=end_turn,
  content_types=[text]
ROUND 4 initial: HTTP 200, elapsed 1.608s, stop_reason=tool_use,
  真实 tool_use.id 存在
ROUND 4 continuation: 客户端 ReadTimeout 45.015s，未收到 HTTP 响应，不计成功
```

结论：真实 call_id continuation 至少有两条单次链路成功，但重复稳定性失败（502/45 秒超时）；未达到“重复验证至少两轮均有效”的验收门槛。

## 3. 长稳定 prefix/cache

固定 system prefix（约 1,768 个 prefix tokens；每次 user suffix 仅变更 probe 编号），临时实例日志中三次请求保持同一 `prefix_fingerprint=63ffaf3cce4b3f77`。实际响应：

```text
CACHE request 5: HTTP 200, elapsed 4.692s,
  input_tokens=1771, output_tokens=32,
  cache_read_input_tokens=0, cache_creation_input_tokens=1768
CACHE request 6: HTTP 200, elapsed 5.367s,
  input_tokens=1771, output_tokens=32,
  cache_read_input_tokens=0, cache_creation_input_tokens=1768
CACHE request 7: HTTP 200, elapsed 2.254s,
  input_tokens=1771, output_tokens=32,
  cache_read_input_tokens=0, cache_creation_input_tokens=1768
```

三次均为有效 HTTP 200，满足“连续 3 个有效请求”的可用性部分；但真实 `cache_read_input_tokens / 上一轮稳定 prefix tokens` 为 `0 / 1768 = 0%`，低于 90%，因此 cache 命中验收失败。`cache_creation_input_tokens` 记录为 1768，未将写入伪装为读取命中。

## 4. timeout 原因记录

本次 timeout 出现在上游 Responses 请求完成前：客户端在 45 秒/50 秒读取窗口内未收到完整响应，临时代理进程仍存活；代理日志能看到相同 prefix fingerprint 的请求已构建，但没有可供客户端解析的 HTTP 200 usage。该请求因此只计为失败证据，**不计入 cache 命中或有效 HTTP 200**。此前报告中的 40 秒/120 秒 timeout 也遵循同一规则：不能把无响应请求计入 cache 统计。

当前证据不足以将 timeout 归因到本地路由代码；已确认上游健康探测 HTTP 200，但上游在部分长/continuation 请求上的响应耗时或响应完成存在不稳定性。

## 5. 清理与生产健康

```text
临时 PID 1142685：已 SIGTERM，进程正常退出
最终 11449：无监听
11441：仍由既有 cc-proxy PID 3603271 监听；health HTTP 200
```

本轮未改动源代码，仅新增本验收记录。最终判定：**CONDITIONAL，不能批准生产部署；P1 需要继续处理上游/旁路稳定性后再验收。**
