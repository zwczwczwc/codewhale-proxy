# Responses Cache A/B 前置审计

- 审计时间：2026-08-06 08:25:41 +08:00
- 工作目录：`/root/projects/codewhale-proxy/source`
- 目标分支：`feat/gpt-responses-transport`
- 审计性质：只读前置核实；未修改源代码、生产配置、systemd，未重启生产服务，未向生产 `11441` 发送业务请求。
- 脱敏规则：本文只记录命令、状态、HTTP 状态码和 exit code；不记录 token、Authorization 值、prompt、tool schema、响应正文。

## 1. 执行摘要

当前代码和本地质量门已具备执行隔离 A/B 的基础：`target/debug/cc-proxy` 存在，Rust 测试 77/77 通过，fmt、clippy 和 `git diff --check` 均通过。目标上游 `clawbot:11434` 当前 DNS/TCP/`/v1/models` 可达，但其 `GET /v1/responses` 返回 404；GET 结果不能作为 Responses POST 能力结论。生产 `11441` 正常运行且健康检查 HTTP 200，临时 `11449` 当前未监听。

认证方面，代码支持通过 `DEEPSEEK_API_KEY` 运行时环境变量注入，并由 HTTP client 生成 Authorization header；但当前审计没有读取任何真实值，也没有发现一个可确认的、可安全转交给临时 A/B 进程的非生产真实凭证。故可安全执行“占位/无认证的启动和健康检查”，不能据此宣称已完成真实 Responses A/B；真实业务 A/B 在认证注入得到明确批准或提供隔离测试凭证前保持 BLOCKED。

## 2. 输入文档交叉核对

已完整读取以下 5 个输入文件：

1. `CC-PROXY-RESPONSES-FINAL-IMPLEMENTATION-PLAN.md`
2. `CC-PROXY-RESPONSES-CONTEXT-RECOVERY.md`
3. `RESPONSES-MULTITURN-PREFIX-CACHE-VALIDATION.md`
4. `RESPONSES-PREFIX-ALIGNMENT-CODE-REVIEW.md`
5. `RESPONSE-E2E-VALIDATION-REMEDIATION-2.md`

交叉一致的关键事实：

| 主题 | 证据 | 判断 | 置信度 |
|---|---|---|---|
| Responses 路由 | `config.toml` 中仅 `gpt-5.6-luna` 设置 `wire_api = "responses"`；输入文档描述 DeepSeek/GLM/Kimi 继续 Chat | 路由边界明确 | high |
| relocation 缺陷 | 多份报告均记录旧路径修改 `messages.last_mut()`，导致多轮公共历史 item 改变 | 需要使用隔离旁路验证；代码/报告证据一致 | high |
| 本地修复现状 | 现有测试包含 Responses relocation 稳定性和三层 hash 测试；本次 `cargo test --locked` 为 77/77 | 当前工作树已有增量实现 | high |
| 上游 cache 结论 | 文档记录过 cache read、cache creation、502 和 timeout，且明确不可混为一谈 | 真实 A/B 必须分别记录这些状态 | high |
| 生产边界 | 输入文档统一要求只用 `127.0.0.1:11449` 临时实例和 `clawbot:11434`，不触碰 `11441` | 本审计遵守 | high |

注意：输入文档之间存在历史时间点差异（测试数 69/76/77、早期 `GET /v1/responses` 结果不同）。本审计以本次命令实际输出为准，不把历史报告当作当前 live 状态。

## 3. 工作树、二进制和质量门

### 3.1 工作树

实际命令：

```text
git status --short --branch
git diff --stat
git diff --check
```

结果：

```text
exit code: 0
branch: feat/gpt-responses-transport
status: 24 个已修改源码/配置文件；多个未跟踪 Markdown、Responses 源码文件
工作树: git diff --check 无输出（通过）
```

工作树不是干净树。后续 coder 必须保留既有改动，不能 reset、clean 或覆盖这些文件。

### 3.2 二进制

实际检查：

```text
test -x target/debug/cc-proxy
stat target/debug/cc-proxy
file target/debug/cc-proxy
```

结果：

```text
exit code: 0
存在且可执行：target/debug/cc-proxy
ELF x86-64，debug_info，未 strip
size: 124822104 bytes
mtime: 2026-08-05 20:57:18 +0800
```

二进制可用于临时旁路，但应先确认它由当前工作树构建；本审计没有替换生产二进制。

### 3.3 质量门

实际执行：

```text
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

结果：

```text
cargo test --locked: exit 0；77 passed, 0 failed
cargo fmt --all -- --check: exit 0
cargo clippy --all-targets --all-features -- -D warnings: exit 0
git diff --check: exit 0
```

## 4. 端口、进程和上游可达性

### 4.1 端口矩阵

| 目标 | 实际检查 | 当前结果 | A/B 含义 |
|---|---|---|---|
| `127.0.0.1:11441` | `ss -ltnp 'sport = :11441'` | `0.0.0.0:11441 LISTEN`，PID 3603271，`/usr/local/bin/cc-proxy` | 生产入口存在；只允许健康检查，不允许本实验业务请求 |
| `127.0.0.1:11449` | `ss -ltnp 'sport = :11449'`、curl health | 无监听；curl exit 7 / HTTP 000 | 可作为临时 A/B 端口；当前没有临时实例 |
| `clawbot:11434` | `getent hosts`、TCP 探测、`GET /v1/models` | 解析 `100.64.0.1`；TCP open；HTTP 200 | 上游网络边界可达 |
| `127.0.0.1:11434` | 本任务禁止使用 | 未调用 | 明确排除本机 Ollama |

### 4.2 生产健康检查

实际命令：

```text
curl -sS --max-time 5 http://127.0.0.1:11441/health
```

脱敏结果：

```text
exit code: 0
HTTP 200
body keys/status: service=cc-proxy, status=ok
```

这只是生产健康证据，不是生产 Responses 业务验收证据。

### 4.3 上游检查

实际命令：

```text
getent hosts clawbot
(timeout 3 bash -c '</dev/tcp/clawbot/11434')
curl -sS --max-time 8 http://clawbot:11434/v1/models
curl -sS --max-time 8 http://clawbot:11434/v1/responses
```

脱敏结果：

```text
getent hosts: exit 0；clawbot -> 100.64.0.1
TCP 11434: open
GET /v1/models: exit 0；HTTP 200
GET /v1/responses: exit 0；HTTP 404
```

`GET /v1/responses` 的 404 不能否定 Responses，因为 Responses 通常需要 POST；它只能证明该 GET 探测不是能力验收。当前没有在本审计中执行业务 POST，因此不虚构 Responses POST 结论。

## 5. 认证和进程边界审计

### 5.1 代码支持的注入方式

只读检查 `src/config.rs`、`src/client.rs` 得到：

```text
DEEPSEEK_API_KEY -> Config.api_key
client 请求时生成 Authorization: Bearer <运行时值>
默认值为占位字符串 not-needed（不能视为真实认证）
```

这是“进程启动时环境变量注入”，不是从文件或命令行打印凭证。推荐的安全形态是：

```text
DEEPSEEK_API_KEY=<由受控运行时注入，不在命令行/日志中展开> \
LISTEN_ADDR=127.0.0.1:11449 \
ESWITCH_URL=http://clawbot:11434 \
MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml \
target/debug/cc-proxy
```

本文不填入真实值，不执行打印环境变量的命令。

### 5.2 当前能确认和不能确认的边界

| 项目 | 结果 | 依据 |
|---|---|---|
| 代码是否有认证注入点 | 是 | `src/config.rs` / `src/client.rs` 静态检查 |
| 是否读取真实 token | 否 | 本审计所有检查仅确认字段/路径，不读取值 |
| 是否把 token 写入报告或日志 | 否 | 命令输出经过脱敏；未输出 systemd `Environment` 值 |
| 是否有可确认的隔离真实测试凭证 | 否 | 当前 shell 未发现可用 API key；systemd 环境仅做存在性/配置结构检查，不提取值 |
| 占位值能否证明上游认证成功 | 否 | `not-needed` 只能启动代理；不能作为真实上游授权证据 |
| 是否可以安全开始真实 A/B | **阻塞** | 需要受控注入一个明确授权的隔离测试凭证，或由上游明确允许无认证测试 |

systemd 服务 `cc-proxy.service` 当前 active；服务文件包含多项 `Environment=`，但本审计只查看结构，不读取其值，也不复用生产环境注入到临时实例。这是刻意的凭证边界。

## 6. A/B 执行可行性判断

更正：本节早期版本曾使用相反的 A/B 标签，该旧标签已废弃，不得作为实验定义。唯一有效定义与任务原始定义一致：A 是直连 `http://clawbot:11434/v1/responses`（noproxy）；B 是临时 `http://127.0.0.1:11449/v1/messages` Anthropic 入口。两组都禁止触碰生产 `11441`；A 也不得改用本机 `127.0.0.1:11434`。

| 维度 | A：上游直连（noproxy） | B：临时 Anthropic 入口 | 当前判断 |
|---|---|---|---|
| 进程/端口 | 不需本地常驻进程 | 可用现有 debug binary，绑定 `127.0.0.1:11449` | A 可执行；B 可执行性取决于直连客户端和授权 |
| 网络 | 代理到 `http://clawbot:11434` | TCP 已 open，DNS 已解析 | 两组网络前置通过 |
| 路由 | 需直接 POST `/v1/responses`，不允许 Chat fallback | 需验证 `/v1/messages`→Responses `/v1/responses`，不允许 Chat fallback | A 待执行；B 未执行 |
| 认证 | 可用运行时 env 注入，不能读取生产值 | 需要同一隔离授权凭证/明确无认证 | 两组真实业务均被认证前置阻塞 |
| cache 对照 | 可记录上游原始 usage（不输出正文） | 可记录临时入口生成的静态/历史/wire hash 和上游 usage | 设计可行，当前无实测 |
| tool continuation | 需构造等价 Responses continuation | 需临时端口上的 Anthropic 请求链路 | 需认证后执行 |
| streaming | 需 A 的 Responses SSE | 需 B 的 `/v1/messages` SSE | 需认证后执行 |
| 生产影响 | 绑定 loopback、可短命运行，理论上隔离 | 不改生产 | 可控，但必须有人审阅启动授权 |

### 6.1 明确结论

- **A 组（直连 noproxy）：条件可执行，但当前不能安全完成真实业务验收。** 缺少已确认授权的隔离认证注入。本地单元测试和二进制检查不能替代真实 upstream POST。
- **B 组（临时 Anthropic 入口）：条件可执行，但当前不能安全完成真实业务验收。** 11449 当前未监听，尚未启动临时进程；没有安全凭证和授权就不应发送业务 POST。
- **因此本卡的 blocker：认证凭证边界，而不是已证实的代码编译、端口或 TCP 故障。** 在凭证未明确注入前，不应让 coder 宣称 A/B、cache hit、tool continuation 或 Responses POST 已通过。

## 7. 可复用的脱敏命令清单

以下命令不会打印 token、prompt、tool schema 或响应正文；可作为后续 coder 的前置/清理检查：

```bash
cd /root/projects/codewhale-proxy/source

git status --short --branch
git diff --stat
git diff --check

cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings

if test -x target/debug/cc-proxy; then
  stat -c 'binary=%n size=%s mtime=%y' target/debug/cc-proxy
else
  echo 'binary=missing'
fi

ss -ltnp 'sport = :11441'
ss -ltnp 'sport = :11449'

curl -sS --max-time 5 -o /dev/null \
  -w '11441 health HTTP %{http_code}, curl_exit=%{exitcode}\n' \
  http://127.0.0.1:11441/health

getent hosts clawbot
timeout 3 bash -c '</dev/tcp/clawbot/11434'
curl -sS --max-time 8 -o /dev/null \
  -w 'clawbot models HTTP %{http_code}, curl_exit=%{exitcode}\n' \
  http://clawbot:11434/v1/models

curl -sS --max-time 3 -o /dev/null \
  -w '11449 health HTTP %{http_code}, curl_exit=%{exitcode}\n' \
  http://127.0.0.1:11449/health
```

启动/清理命令模板（仅在获得隔离认证授权后使用；不把凭证写入命令历史）：

```bash
# 用受控运行时注入 DEEPSEEK_API_KEY；不要把真实值替换进文档或 shell 命令文本
LISTEN_ADDR=127.0.0.1:11449 \
ESWITCH_URL=http://clawbot:11434 \
MODEL_CONFIG_PATH=/root/projects/codewhale-proxy/source/config.toml \
RUST_LOG=info \
DEEPSEEK_API_KEY='<受控注入占位，不应出现在审计记录>' \
./target/debug/cc-proxy

# 测试后只确认进程和监听已消失，不打印服务日志正文
ss -ltnp 'sport = :11449'
```

注意：上面的启动模板不是本次已执行的真实 A/B 结果；它只是后续授权后的可复用边界模板。

## 8. 建议的分阶段执行路线

### Phase 1：认证和隔离准备（约 15–30 分钟）

1. 由负责人确认一个仅用于 A/B 的隔离测试凭证，或书面确认上游允许无认证测试。
2. 确认凭证只能通过受控环境注入，禁止写入 Markdown、命令历史、源码和日志。
3. 再次执行 11441 health、11449 无监听、clawbot TCP/模型健康检查。

退出条件：认证注入方式得到明确批准；否则停在 blocker。

### Phase 2：短命 A/B 实验（约 30–60 分钟）

1. B 仅监听 `127.0.0.1:11449`，执行普通 Anthropic、xhigh、tool continuation、stream。
2. A 直接请求 `clawbot:11434/v1/responses`，使用同一请求形状和隔离认证边界。
3. 只记录 HTTP 状态、耗时、usage 字段名/数值、hash 摘要和错误类别；不记录正文。
4. 将 `cache_read=0`、`cache_creation>0`、502/504、timeout 分开统计。

退出条件：A/B 都有可复核的命令 exit code 和脱敏结果；任意认证/上游错误都保留为失败证据，不改写成 cache 结论。

### Phase 3：清理与判定（约 10–20 分钟）

1. SIGTERM 清理临时实例。
2. 确认 `11449` 无监听。
3. 只做 `11441/health` 健康检查，确认仍 HTTP 200；不做生产业务请求。
4. 形成 A/B 对比表并由 reviewer 判定是否关闭 P1。

## 9. 风险与缓解

| 风险 | 当前状态 | 缓解 |
|---|---|---|
| 误把生产 `11441` 当实验端口 | 11441 正在监听 | 实验只允许 `127.0.0.1:11449`；11441 只做 health |
| 误用本机 `127.0.0.1:11434` | 本审计未使用 | 所有上游命令显式使用 `clawbot:11434` |
| token 泄露到命令/日志 | 当前未发生 | 受控 env 注入；不读取 systemd 值；日志/报告只保留脱敏摘要 |
| 把 GET 404 当 Responses 不支持 | 已观测 GET 404 | 只用授权的 POST 作为能力证据；GET 只记为探测结果 |
| 把 cache creation 当 cache read | 历史报告已出现该风险 | 独立记录 `cache_read_input_tokens` 与 `cache_creation_input_tokens` |
| 上游 502/504/timeout 被归因于 prefix | 历史测试已出现 | 单独错误类别统计，不能计为 cache miss 或命中 |
| 工作树被实验覆盖 | 当前有 24 个 modified 文件和未跟踪文件 | 不 reset/clean；临时日志写 `/tmp`，不改仓库 |
| 临时实例残留 | 11449 当前无监听 | 实验后 SIGTERM + `ss` 复核；发现监听即视为清理失败 |

## 10. 最终判定和 blocker

```text
本地构建/测试前置：PASS
生产 11441 健康：PASS（仅 health，未做业务请求）
临时 11449 状态：PASS（当前无监听，可作为隔离端口）
clawbot:11434 网络可达：PASS（DNS/TCP/models HTTP 200）
clawbot GET /v1/responses：HTTP 404（非能力结论）
安全真实认证注入：BLOCKED
A 组真实业务验收：BLOCKED（认证授权前）
B 组真实业务验收：BLOCKED（认证授权前）
生产部署批准：BLOCKED
```

阻塞原因精确表述：当前可确认代码层面支持 `DEEPSEEK_API_KEY` 运行时注入，但没有可安全确认的隔离真实凭证，且不能读取/复用生产 systemd 环境中的认证值。占位值只能证明临时进程可以启动，不能证明上游 Responses POST、tool continuation、SSE 或 cache read。需要负责人提供隔离测试凭证的受控注入方案，或明确授权无认证 POST；在此之前禁止执行真实 A/B 业务请求，禁止修改生产服务。
