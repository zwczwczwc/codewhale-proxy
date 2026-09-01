# cc-connect + cc-proxy P0/P1 修复恢复清单

## 首先阅读

- `CC-CONNECT-CC-PROXY-P0-P1-REPAIR-PLAN-2026-08-09.md`
- `/root/projects/codewhale-proxy/source/src/client.rs`
- `/root/projects/codewhale-proxy/source/src/routes/messages.rs`
- `/root/projects/codewhale-proxy/source/src/responses/request.rs`
- `/root/projects/codewhale-proxy/source/src/responses/stream.rs`
- `/root/projects/codewhale-proxy/source/src/responses/response.rs`
- `/tmp/cc-connect-source/core/engine.go`
- `/tmp/cc-connect-source/config/config.go`
- `/tmp/cc-connect-source/cmd/cc-connect/main.go`

## Git 基线

- 仓库：`/root/projects/codewhale-proxy/source`
- 当前 HEAD：`1b57681`
- 现有工作树包含大量未跟踪 Markdown 和 `tools/`；不得 reset、clean 或覆盖。
- 当前生产 cc-proxy：`/usr/local/bin/cc-proxy`，监听 `0.0.0.0:11441`，systemd MainPID 必须保持不变，除非用户后续明确通知部署。

## 当前现场事实

- cc-connect：`v1.4.1`，`/usr/local/bin/cc-connect`，配置 `/home/claude/.cc-connect/config.toml`。
- 当前日志上限环境变量：`CC_LOG_MAX_SIZE=10485760`。
- `max_turn_time_mins=60` 当前位于 `[projects.agent.options]`，按 v1.4.1 源码不会接线；修复目标是顶层配置。
- 当前 display 隐藏 thinking/tool；用户要求只显示最终正文，因此 Feishu 使用 `progress_style=legacy`（或默认值），不启用 compact/中间进度。
- cc-proxy 生产二进制与当前 release artifact SHA-256 相同，但本轮禁止替换运行中二进制。

## 禁止事项

- 不覆盖 `/usr/local/bin/cc-proxy`。
- 不停止或重启 `cc-proxy.service`。
- 不修改 `/etc/cc-proxy/config.toml`。
- 不把 11441 当作测试端口。
- 不 reset/clean/checkout 覆盖现有工作树。
- 不把 health、静态单测或旧报告当作真实业务验收。

## 第一轮命令

```bash
git status --short --branch
systemctl show cc-proxy.service -p MainPID -p ExecStart -p NRestarts --no-pager
sha256sum /usr/local/bin/cc-proxy target/release/cc-proxy
python3 - <<'PY'
import tomllib
p='/home/claude/.cc-connect/config.toml'
d=tomllib.loads(open(p, encoding='utf-8').read())
print(d.get('max_turn_time_mins'), d.get('idle_timeout_mins'))
PY
```

## 验收门

- cc-connect 配置解析和日志大小上限真实验证；
- cc-connect 短 turn 只发送最终正文并最终 `turn complete`；
- cc-proxy 源码测试、fmt、check、clippy、release build 通过；
- 候选 artifact 路径和 SHA-256 已记录；
- 生产 cc-proxy MainPID、ExecStart、SHA、11441 listener 未改变；
- 未经用户后续通知，不报告 cc-proxy 已部署。
