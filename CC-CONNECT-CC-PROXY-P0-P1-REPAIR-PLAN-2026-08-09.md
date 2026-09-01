# cc-connect + cc-proxy P0/P1 修复计划

**日期**：2026-08-09

## Goal

修复 cc-connect 的超时配置接线和中间进度不可见问题；为 cc-proxy Responses 路径补齐请求关联、上游响应、首字节、terminal/EOF/error 可观测性；完成源码与候选构建验证，但不替换或重启当前生产 `cc-proxy.service`。

## 当前边界

- cc-connect：`v1.4.1`，生产二进制为 `/usr/local/bin/cc-connect`，配置为 `/home/claude/.cc-connect/config.toml`，日志为 `/home/claude/.cc-connect/logs/cc-connect.log`。
- cc-proxy：生产监听 `0.0.0.0:11441`，systemd `ExecStart=/usr/local/bin/cc-proxy`；源码目录为本仓库，生产二进制与当前 `target/release/cc-proxy` 需保持可核对。
- 禁止事项：不覆盖 `/usr/local/bin/cc-proxy`，不停止/重启 `cc-proxy.service`，不修改 `/etc/cc-proxy/config.toml`，不 reset/clean 现有工作树。

## P0：cc-connect 超时配置接线

1. 备份当前 cc-connect 配置和 unit 元数据。
2. 将 `max_turn_time_mins = 60` 从 `[projects.agent.options]` 移到 TOML 顶层；该字段依据 cc-connect v1.4.1 的 `config/config.go::Config` 和 `cmd/cc-connect/main.go` wiring。
3. 显式设置顶层 `idle_timeout_mins = 120`，避免依赖默认值漂移。
4. 保留并核验 `CC_LOG_MAX_SIZE=10485760`（10 MiB）文件日志上限；不允许日志无限增长。
5. 仅在确认当前 Claude turn 空闲后重载/重启 cc-connect；不触碰 cc-proxy。

## P1：cc-connect 正文-only 显示

1. 保持 cc-connect 的协议、Claude stdin、ANTHROPIC_BASE_URL、工具执行和 session resume 不变。
2. 保持 `[display] thinking_messages = false` 与 `tool_messages = false`，只向 Feishu 发送最终 assistant 正文。
3. Feishu `progress_style` 使用 `legacy`（或省略并使用默认值）；不启用 `compact`，因为用户不希望看到工具/推理进度。
4. 说明边界：display 配置不会让长 turn 变快，也不会消除等待期间的“无正文”状态；长 turn 保护由 P0 超时，故障定位由 cc-proxy 观测承担。

## P1：cc-proxy 候选可观测性

仅修改源码并构建候选 artifact，不部署：

- 以现有 Anthropic `msg_id` 作为 request correlation id；
- Responses 请求日志记录 request id、upstream HTTP status、headers 到达耗时；
- Responses stream 记录 first byte、terminal event、terminal usage、上游 stream error、EOF without terminal、下游断开；
- 非流式 Responses 记录请求完成/失败耗时；
- 保持现有 Responses input/history/tools/call_id/cache wire 不变；不增加隐式 Chat fallback；本轮不增加有副作用的自动重试。

## 测试与验收

### cc-connect

- TOML 解析：顶层 `max_turn_time_mins=60`、`idle_timeout_mins=120`；agent options 中不再存在同名超时字段。
- 启动后日志确认配置加载、日志 `max_size=10485760`；确认新 MainPID、Claude session 可 resume。
- cc-connect 短 turn 只发送最终正文并最终 `turn complete`；
- 确认生产 `cc-proxy.service` MainPID、ExecStart、binary SHA、监听 11441 均未改变。

### cc-proxy

- RED/GREEN：为新增 telemetry helper/terminal 状态写单元测试，先观察失败，再实现。
- `cargo fmt --check`
- `cargo check --locked`
- `cargo test --locked --all-targets`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `git diff --check`
- `cargo build --release`，记录候选 artifact 绝对路径和 SHA-256。
- 不启动生产候选、不监听 11441；如需真实旁路，只能使用 127.0.0.1:11449，且本计划不自动执行。

## 回滚

- cc-connect：恢复配置备份，随后按 `stop → start` 重启 cc-connect；不触碰 cc-proxy。
- cc-proxy：本轮只保留源码 diff 和候选 artifact；未替换生产二进制，因此无需生产回滚。
