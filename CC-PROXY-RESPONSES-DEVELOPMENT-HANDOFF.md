# cc-proxy Responses 后续开发恢复清单

> 用途：上下文压缩后，下一会话开始直接开发前的唯一恢复入口。
>
> 重要：本文件记录的是 **2026-08-06 当前实际状态**。`CC-PROXY-RESPONSES-CONTEXT-RECOVERY.md` 是较早阶段的历史恢复文档，其中的 master/无源码改动等基线已经过时；若与本文件冲突，以本文件和实时 Git/进程检查为准。

## 1. 当前事实基线

### 项目

```text
源码目录：/root/projects/codewhale-proxy/source/
当前分支：feat/gpt-responses-transport
工作树：有大量未提交 Responses 改造和文档改动
```

禁止：

```text
git reset --hard
git clean
覆盖或回滚既有未提交改动
```

开始任何开发前先执行：

```bash
cd /root/projects/codewhale-proxy/source
git branch --show-current
git status --short --branch
git diff --stat
git diff --check
```

### 生产隔离

```text
生产 cc-proxy：0.0.0.0:11441
当前生产 PID：以实时 ss 输出为准（此前为 3603271）
临时旁路：127.0.0.1:11449
当前状态：11449 已清理；11441 仍在监听
```

开发/验收阶段禁止：

```text
修改 /etc/cc-proxy/
修改 systemd cc-proxy.service
停止、重启或替换生产 11441
替换 /usr/local/bin/cc-proxy
```

### 正确上游

```text
上游：http://clawbot:11434
解析：100.64.0.1
```

禁止把本机以下地址当作 Responses 上游：

```text
http://127.0.0.1:11434
```

所有直接业务请求必须：

```http
Authorization: Bearer [REDACTED]
Content-Type: application/json
```

缺少 Authorization 的 HTTP 401 不能作为协议或 cache 结论。

## 2. 已完成的实现

当前工作树已经包含以下实现，下一会话不要从零重新设计：

```text
模型级 WireApi：gpt-5.6-luna → Responses；其他 profile 默认 Chat Completions
Anthropic /v1/messages → Responses /v1/responses
user 文本 → input_text
assistant 历史文本 → output_text
tool_use → function_call
tool_result → function_call_output
真实 call_id continuation
Responses 非流式转换
Responses SSE → Anthropic SSE
CODEMERMAFROST_RELOCATE 的 Responses synthetic tail
static_prefix_hash
history_prefix_hash
wire_input_hash
Responses usage/cache/status telemetry
```

核心源码：

```text
src/config.rs
src/routes/messages.rs
src/client.rs
src/responses/request.rs
src/responses/types.rs
src/responses/response.rs
src/responses/stream.rs
src/reasoning/relocate.rs
```

Chat 路径必须保持：

```text
DeepSeek / GLM / Kimi → /v1/chat/completions
```

Responses 失败不得静默 fallback 到 Chat。

## 3. 最新真实 A/B 证据

权威报告：

```text
/root/projects/codewhale-proxy/source/RESPONSES-CACHE-CONTINUATION-AB-LIVE-RESULT.md
```

测试结构：

```text
A：直连 http://clawbot:11434/v1/responses
B：127.0.0.1:11449/v1/messages → cc-proxy → clawbot:11434/v1/responses
```

固定前缀：

```text
PREFIX_CHARS=172520
input_tokens=30453
prefix_hash=ed8b383f0f341b3e
```

A 组：

```text
request 1: cache_read=0,     cache_creation=30450
request 2: cache_read=0,     cache_creation=30450
request 3: cache_read=30450, cache_creation=0
request 4: cache_read=30450, cache_creation=0
```

B 组：

```text
request 1-4: cache_read=30450, cache_creation=0
```

命中率：

```text
30450 / 30453 ≈ 99.9902%
```

注意：A 组先完成 cache warmup，B 组是在同一上游热缓存状态下运行；这证明 cc-proxy 能复用 Responses cache，但不是冷缓存隔离实验。

真实 tool continuation：

```text
A 组独立新 call_id 链路：3/3 HTTP 200 → completed
B 组独立新 tool_use.id 链路：3/3 HTTP 200 → end_turn
```

Streaming：

```text
A 原生 Responses：HTTP 200，观察到 response.completed
B cc-proxy Anthropic SSE：HTTP 200，观察到 message_stop
```

测试后：

```text
11449 已清理
11441 健康 HTTP 200
```

此前的 timeout/502 仍作为间歇性监控历史，不得在新会话中被删除或改写成“从未发生”。

## 4. 必须阅读的文件顺序

### 第一层：先读恢复入口和最终方案

```text
/root/projects/codewhale-proxy/source/CC-PROXY-RESPONSES-DEVELOPMENT-HANDOFF.md
/root/projects/codewhale-proxy/source/CC-PROXY-RESPONSES-FINAL-IMPLEMENTATION-PLAN.md
```

目的：获得当前状态、最终设计、禁止项、TDD 顺序和验收门槛。

### 第二层：读最新真实结果和失败历史

```text
/root/projects/codewhale-proxy/source/RESPONSES-CACHE-CONTINUATION-AB-LIVE-RESULT.md
/root/projects/codewhale-proxy/source/RESPONSE-E2E-VALIDATION-REMEDIATION-2.md
/root/projects/codewhale-proxy/source/RESPONSE-E2E-VALIDATION-REMEDIATION-1.md
/root/projects/codewhale-proxy/source/RESPONSE-E2E-VALIDATION-REPORT.md
```

目的：区分：

```text
已经通过的真实协议/Cache A/B
早期 assistant input_text 协议错误
历史 timeout/502
cache_creation 与 cache_read
```

### 第三层：读根因和实现审查

```text
/root/projects/codewhale-proxy/source/RESPONSES-PREFIX-ALIGNMENT-CODE-REVIEW.md
/root/projects/codewhale-proxy/source/RESPONSES-MULTITURN-PREFIX-CACHE-VALIDATION.md
/root/projects/codewhale-proxy/source/RESPONSES-PREFIX-CACHE-REVIEW.md
/root/projects/codewhale-proxy/source/RESPONSES-PREFIX-CACHE-OPTIMIZATION-PLAN.md
```

目的：理解 `messages.last_mut()` mutation 根因、synthetic tail、三层 hash 和多轮公共 prefix 规则。

### 第四层：读早期设计和历史恢复文档

```text
/root/projects/codewhale-proxy/source/GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md
/root/projects/codewhale-proxy/source/CC-PROXY-RESPONSES-CONTEXT-RECOVERY.md
```

目的：补充原始协议映射、参考项目、初始验收矩阵和历史文件地图。

注意：第二份是历史基线，必须以本文件、当前 Git 和最新 A/B 报告纠正其中的旧状态。

### 第五层：读代码

```text
/root/projects/codewhale-proxy/source/Cargo.toml
/root/projects/codewhale-proxy/source/Cargo.lock
/root/projects/codewhale-proxy/source/config.toml
/root/projects/codewhale-proxy/source/src/config.rs
/root/projects/codewhale-proxy/source/src/routes/messages.rs
/root/projects/codewhale-proxy/source/src/client.rs
/root/projects/codewhale-proxy/source/src/responses/request.rs
/root/projects/codewhale-proxy/source/src/responses/types.rs
/root/projects/codewhale-proxy/source/src/responses/response.rs
/root/projects/codewhale-proxy/source/src/responses/stream.rs
/root/projects/codewhale-proxy/source/src/reasoning/relocate.rs
/root/projects/codewhale-proxy/source/src/anthropic/types.rs
/root/projects/codewhale-proxy/source/src/anthropic/converter.rs
/root/projects/codewhale-proxy/source/src/openai/types.rs
/root/projects/codewhale-proxy/source/src/openai/converter.rs
/root/projects/codewhale-proxy/source/src/sse/stream.rs
```

目的：开发前确认真实调用链，尤其不能把 Responses 代码改回 Chat 路径，也不能让 Chat 逻辑复用 Responses 专用字段。

### 第六层：参考项目（需要修改协议/状态机时才阅读）

```text
mxyhi/token_proxy
commit: 6bed3d1ebbbb44c06833d37b34b2ebe49cc8d8a2
https://github.com/mxyhi/token_proxy

https://github.com/tangsipeng/openai-responses-anthropic-proxy
https://github.com/Lokesh-Chimakurthi/rosetta-llm
https://github.com/musistudio/claude-code-router
```

边界：参考实现模式，不整体替换 cc-proxy；未经许可证确认不复制代码。

## 5. 开发前实时检查

新会话必须先执行：

```bash
cd /root/projects/codewhale-proxy/source
git branch --show-current
git status --short --branch
git diff --stat
git diff --check
ss -ltnp '( sport = :11449 or sport = :11441 )'
getent hosts clawbot
cargo test --all-targets --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

如果发现 11449 有监听：

```text
先确认进程归属；只清理自己启动的临时进程；不得杀生产 11441。
```

## 6. 后续直接开发原则

1. 不重复做已经完成的 Responses 架构设计；先检查当前实现和 diff。
2. 任何新 bug 先写 RED 测试，确认失败原因，再最小 GREEN 修复。
3. 只改根因，不顺手重构 Chat 路径。
4. 每个垂直切片后运行相关测试，再运行全量质量门。
5. 不引入新的常驻代理，不恢复 `proxy.py`、`token_proxy` 或 `cc-switch`。
6. 不默认启用 `previous_response_id`，不伪造 encrypted reasoning replay。
7. 不把 `cache_creation` 当作 `cache_read`。
8. 只把有效 HTTP 200 且 usage 可解析的请求计入 cache 统计。
9. 502/504、timeout、schema 400、401 单独分类。
10. 未明确收到生产部署指令前，不修改生产服务。

## 7. 如果下一阶段是生产发布

研发分支已经有未提交变更；生产发布前还必须单独完成：

```text
审查完整 git diff
确认 commit/PR
备份生产 config.toml 和二进制
确认部署窗口
stop → cp → start（避免 Text file busy）
生产前后 Chat/Responses 回归
保留同步回滚方案
```

当前文件、配置和服务状态不能被描述为“已经生产部署”。

## 8. 压缩后直接开发 Prompt

复制以下 Prompt 作为压缩后的新会话第一条消息：

```text
请先完整阅读并以当前实际状态为准：

1. /root/projects/codewhale-proxy/source/CC-PROXY-RESPONSES-DEVELOPMENT-HANDOFF.md
2. /root/projects/codewhale-proxy/source/CC-PROXY-RESPONSES-FINAL-IMPLEMENTATION-PLAN.md
3. /root/projects/codewhale-proxy/source/RESPONSES-CACHE-CONTINUATION-AB-LIVE-RESULT.md
4. /root/projects/codewhale-proxy/source/RESPONSE-E2E-VALIDATION-REMEDIATION-2.md
5. /root/projects/codewhale-proxy/source/RESPONSES-PREFIX-ALIGNMENT-CODE-REVIEW.md
6. /root/projects/codewhale-proxy/source/RESPONSES-MULTITURN-PREFIX-CACHE-VALIDATION.md
7. /root/projects/codewhale-proxy/source/RESPONSES-PREFIX-CACHE-REVIEW.md
8. /root/projects/codewhale-proxy/source/GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md
9. /root/projects/codewhale-proxy/source/CC-PROXY-RESPONSES-CONTEXT-RECOVERY.md

然后进入 /root/projects/codewhale-proxy/source/，确认当前分支仍为 feat/gpt-responses-transport，先检查 git status/diff、11449/11441 监听状态和当前质量门。不要 reset、clean 或覆盖已有未提交改动。

当前已完成：gpt-5.6 显式走 /v1/responses；DeepSeek/GLM/Kimi 保持 /v1/chat/completions；Responses user=input_text、assistant=output_text、function_call/function_call_output、synthetic tail、三层 prefix hash、非流式/SSE 均已实现；真实 A/B 已证明 30450/30453≈99.9902% cache read，直连和 cc-proxy 旁路 continuation 均 3/3 成功；生产 11441 未修改，临时 11449 已清理。不要把历史 dry-run 报告当作最新真实结果。

接下来开始直接开发，不重新设计架构：严格按 CC-PROXY-RESPONSES-FINAL-IMPLEMENTATION-PLAN.md 执行。若当前没有新的用户指定功能，优先完成开发分支收尾：逐项审查 Responses/Chat 隔离、补齐必要回归测试和文档、运行 cargo test --all-targets --locked、cargo fmt --all -- --check、cargo clippy --all-targets --all-features -- -D warnings、git diff --check，并检查是否需要形成 commit/PR。任何新修改必须先写 RED 测试再实现；不修改生产服务、不部署、不输出凭证。生产发布必须等我单独明确批准。
```
