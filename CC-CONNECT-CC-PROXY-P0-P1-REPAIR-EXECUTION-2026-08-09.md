# cc-connect + cc-proxy P0/P1 修复执行记录

**时间**：2026-08-09

## 已完成

### cc-connect P0/P1 配置候选

已修改：`/home/claude/.cc-connect/config.toml`

- 顶层 `idle_timeout_mins = 120`
- 顶层 `max_turn_time_mins = 60`
- `[display] thinking_messages = false`
- `[display] tool_messages = false`
- Feishu `progress_style = "legacy"`（仅显示最终正文，不启用进度卡）
- 移除 `[projects.agent.options]` 中错误位置的 `max_turn_time_mins`

配置已用 Python `tomllib` 重新解析验证：

```text
top idle_timeout_mins=120
top max_turn_time_mins=60
display thinking/tool=false
project gpu-project
agent.options.max_turn_time_mins=None
feishu progress_style=legacy
```

当前 cc-connect 尚未重启；因此新配置尚未进入运行进程。用户应知道：本轮只准备了配置，尚未宣称运行态修复完成。

日志大小：当前 systemd unit 已有 `CC_LOG_MAX_SIZE=10485760`（10 MiB）。备份中保留了原 unit。由于当前会话环境禁止从 gateway 内执行 systemd 写操作，本轮尚未把 `CC_LOG_MAX_BACKUPS=3` 写入 live unit；现有二进制的默认 max_backups=3，后续应在独立 shell 中显式补上并重载 cc-connect。

## cc-proxy 候选源码

仅修改源码，未修改生产配置、未覆盖生产二进制、未停止/重启 `cc-proxy.service`。

修改文件：

- `src/client.rs`
- `src/responses/request.rs`
- `src/responses/types.rs`
- `src/responses/stream.rs`
- `src/routes/messages.rs`

改动内容：

- 为每个 Responses request 增加内部 request_id；该字段 `serde(skip)`，不会进入 upstream wire、cache hash 或 history。
- 记录 response headers、HTTP status、headers elapsed、content type/length/transfer encoding。
- 记录非流式 Responses 完成耗时和 upstream status。
- 记录 streaming first byte、upstream read error、EOF。
- 记录 terminal event：`response.completed` / `response.incomplete`。
- 记录 EOF without terminal event。
- 保持现有 Responses input/history/tools/call_id/cache wire 不变。
- 未增加隐式 Chat fallback，未增加自动重试。

## 真实构建验证

在 `/root/projects/codewhale-proxy/source`：

- `cargo check --locked`：通过
- `cargo fmt --check`：通过
- `cargo test --locked --all-targets`：通过，101 tests
- `cargo clippy --locked --all-targets --all-features -- -D warnings`：通过
- `git diff --check`：通过
- `cargo build --release`：通过

候选 artifact：

```text
/root/projects/codewhale-proxy/source/target/release/cc-proxy
SHA256: 53aba3bcc29a1bbb6c93b0d005863317d35ec23d1effa783463c0079e4f8dc50
```

生产 binary 仍为：

```text
/usr/local/bin/cc-proxy
SHA256（修改前/当前运行中）：8c43658d854e70c90d11328b5edecd4bc420ddc0ffc208214a69f4babe766884
```

当前生产 `cc-proxy.service`：

- MainPID：3340
- `ExecStart=/usr/local/bin/cc-proxy`
- 监听：`0.0.0.0:11441`
- `NRestarts=0`

以上运行态没有被本轮改变。

## 未完成事项

1. cc-connect 仍需要在独立 shell 执行安全的 stop/start 或 reload 流程，才能让 P0/P1 配置生效；必须先确认当前 turn 空闲。
2. `CC_LOG_MAX_BACKUPS=3` 尚未显式写入 live systemd unit；当前 binary 默认值为 3，但建议后续明确配置。
3. cc-proxy 候选尚未做 11449 旁路真实业务验证；生产 11441 不能作为候选测试端口。
4. cc-proxy 候选尚未部署，必须等待用户后续明确通知。

## 恢复与回滚

cc-connect 备份：

- `/data/backups/cc-connect/config.toml.before-p0-p1-20260809-020416`
- `/data/backups/cc-connect/cc-connect.service.before-p0-p1-20260809-020416`
- `/data/backups/cc-connect/override.conf.before-p0-p1-20260809-020416`

源码工作树保留了原有未跟踪文件，没有 reset/clean。