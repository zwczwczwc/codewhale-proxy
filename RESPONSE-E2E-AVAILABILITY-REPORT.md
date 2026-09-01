# Responses 旁路 E2E / Claude Code / cache 可用性探测报告

- 探测时间：2026-08-05 18:43–18:44（Asia/Shanghai，命令输出以 `date --iso-8601=seconds` 为准）
- 范围：仅本机 `127.0.0.1` 明确地址和本地 CLI/配置只读检查
- 安全边界：未访问公网或生产上游，未发送业务请求，未打印任何 token、环境变量值、Authorization header 或配置 secret；未启动/修改服务，未修改源代码或配置
- 结论：当前不能进行 Responses 旁路 E2E、Claude Code E2E、tool continuation、streaming 或 cache 验收。唯一可达的 `11434` 进程确认是 Ollama，不足以证明目标 eswitch/Responses 能力；旁路端口 `11449` 未监听且连接失败。Claude Code CLI 已安装，但本机存在本地配置入口并配置了凭证字段（仅确认存在性），未在无可确认隔离上游和明确测试授权的情况下调用。

## 1. 本机监听与进程归属

实际命令：

```text
$ date --iso-8601=seconds; ss -ltnp '( sport = :11449 or sport = :11434 or sport = :11435 or sport = :11441 )'
```

exit code：`0`

结果摘要：

```text
2026-08-05T18:43:00+08:00
0.0.0.0:11441 LISTEN -> /usr/local/bin/cc-proxy (pid 3603271)
*:11434 LISTEN -> /usr/local/bin/ollama serve (pid 3429)
11449、11435：未出现在监听结果中
```

为避免把端口归属猜成 eswitch，又执行了只读进程命令行检查：

```text
$ tr '\0' ' ' </proc/3603271/cmdline
```

exit code：`0`；结果：`/usr/local/bin/cc-proxy`

```text
$ tr '\0' ' ' </proc/3429/cmdline
```

exit code：`0`；结果：`/usr/local/bin/ollama serve`

因此：`11441` 是正在运行的 cc-proxy 监听，但不是本任务要求的明确隔离旁路端口；`11434` 明确归属 Ollama，不能据此判定为 eswitch。没有调用 `11441` 的业务 API，也没有调用不明归属端口的业务接口。

## 2. 本地健康/可达性只读探测

### 2.1 目标旁路端口 11449

```text
$ date --iso-8601=seconds; curl --fail --silent --show-error --max-time 3 -o /dev/null http://127.0.0.1:11449/health
```

时间：`2026-08-05T18:43:15+08:00`

exit code：`7`

摘要：`curl: (7) Failed to connect to 127.0.0.1 port 11449`。旁路服务未监听/不可达。

再次检查根路径：

```text
$ date --iso-8601=seconds; curl --fail --silent --show-error --max-time 3 -o /dev/null http://127.0.0.1:11449/
```

时间：`2026-08-05T18:44:28+08:00`

exit code：`7`。未收到 HTTP 响应。

### 2.2 11434（仅确认本机 Ollama 健康，不作为 eswitch 验收）

```text
$ date --iso-8601=seconds; curl --fail --silent --show-error --max-time 3 -o /dev/null http://127.0.0.1:11434/
```

时间：`2026-08-05T18:43:17+08:00`；exit code：`0`

```text
$ date --iso-8601=seconds; curl --fail --silent --show-error --max-time 3 http://127.0.0.1:11434/api/version -o <temporary-file>
```

时间：`2026-08-05T18:44:05+08:00`；exit code：`0`

仅解析响应键名（未输出响应值）：`response_keys=version`。

```text
$ date --iso-8601=seconds; curl --fail --silent --show-error --max-time 3 -o /dev/null http://127.0.0.1:11434/api/tags
```

时间：`2026-08-05T18:44:29+08:00`；exit code：`0`。

这些结果只说明本地 Ollama HTTP 接口可达；由于进程归属已确认是 Ollama，未向其发送 Responses、tools、streaming、tool result 或 cache 业务请求。

## 3. Claude Code CLI 与非生产配置入口

CLI 检查：

```text
$ command -v claude
```

结果：`/usr/bin/claude`

```text
$ claude --version
```

exit code：`0`；结果：`2.1.209 (Claude Code)`。

```text
$ claude --help
```

exit code：`0`。帮助显示支持 `--print`、`--effort <level>`（包含 `xhigh`）、`--settings` 等入口；本次没有执行 prompt/API 调用。

配置文件存在性（只读）：

```text
$ test -e /root/.claude/settings.json
$ test -e /root/.claude.json
$ test -e /root/.config/Claude/claude_desktop_config.json
```

exit code：`0`；结果：

```text
/root/.claude/settings.json=present
/root/.claude.json=present
/root/.config/Claude/claude_desktop_config.json=absent
```

对 `/root/.claude/settings.json` 仅解析 JSON 的键路径，不打印值：

```text
$ python3 <只输出 JSON 键路径的检查脚本>
```

exit code：`0`；结果包含 `env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_API_KEY`、`env.ANTHROPIC_AUTH_TOKEN` 等字段，且 `CLAUDE_SETTINGS_JSON=parse_ok`。

随后仅报告配置字段状态，不打印值：

```text
ANTHROPIC_BASE_URL_CONFIG=localhost
ANTHROPIC_API_KEY_CONFIG=nonempty
ANTHROPIC_AUTH_TOKEN_CONFIG=nonempty
```

当前 shell 环境变量存在性检查（值均未打印）：

```text
ANTHROPIC_API_KEY=False
ANTHROPIC_AUTH_TOKEN=False
ANTHROPIC_BASE_URL=False
CLAUDE_CODE_USE_BEDROCK=False
CLAUDE_CODE_USE_VERTEX=False
OPENAI_API_KEY=False
OPENAI_BASE_URL=False
ESWITCH_URL=False
CC_PROXY_URL=False
```

说明：Claude Code 可执行文件和本地配置入口存在；配置文件中存在 localhost base URL 及非空凭证字段，但本次没有读取其值，也没有将其视为已验证可用的“非生产测试凭证”。`claude config list` 在 30 秒内超时（exit code `124`），且未保留/输出其内容：

```text
$ claude config list >/tmp/... 2>/tmp/...
```

## 4. 未执行的 E2E/cache 项目及原因

以下请求均**未执行**，没有虚构 HTTP 200、Responses 事件、tool result、cache hit 或日志：

- gpt-5.6 非流式 Responses 文本
- `reasoning.effort=xhigh`
- Responses function tools
- tool result 至少两轮续接
- Responses streaming 文本/工具事件
- Claude Code 普通、xhigh、工具、多轮、流式 E2E
- 长稳定前缀四次请求及 `cached_tokens` 命中率
- eswitch `/v1/responses` 日志确认

阻塞项：

1. 任务要求的本机隔离旁路 `127.0.0.1:11449` 未监听，curl 连接 exit `7`。
2. 现有 `127.0.0.1:11434` 归属 `/usr/local/bin/ollama serve`，不是可确认的 eswitch；按安全边界不能将其当作目标上游并调用业务接口。
3. 没有可安全注入且已授权的临时非生产凭证/测试窗口确认。虽然 Claude 配置字段存在非空值，但凭证值未读取、未验证、未使用。
4. `claude config list` 超时（exit `124`），不过 CLI 版本和配置键路径已经通过独立只读命令确认。

## 5. 人工需要提供的最小解锁条件

在继续 E2E/cache 前，需要人工明确提供并确认：

- 隔离实例的监听地址，优先 `127.0.0.1:11449`，以及该实例确实是本任务的临时 cc-proxy/eswitch 测试链路；
- 隔离上游 eswitch endpoint（若非本机，不应直接提供生产地址），并确认允许的测试范围；
- 仅用于该隔离实例的临时测试凭证注入方式（环境变量名或受控 secret mount；不需要在聊天/报告中提供明文）；
- Claude Code E2E 的明确批准窗口和允许的无害 prompt/tool 测试目录；
- 若要验收 cache：固定、超过 1024 token 的 instructions/tools 测试向量，以及允许连续至少四次调用的隔离 cache 实例。

在这些条件满足前，正确状态是 **BLOCKED / 未验收**，而不是“通过”。

## 6. 文件与变更记录

本次仅新增本报告文件；未修改源代码、生产配置、运行中服务或常驻代理。报告写入前仓库已有修改保持原样，未覆盖或回滚：

```text
/root/projects/codewhale-proxy/source/RESPONSE-E2E-AVAILABILITY-REPORT.md
```
