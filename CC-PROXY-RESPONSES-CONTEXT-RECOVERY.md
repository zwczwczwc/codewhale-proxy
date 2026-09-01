# cc-proxy gpt-5.6 Responses 改造：新会话恢复上下文清单

> 用途：新会话开始开发前，按本文件顺序恢复上下文。本文只列“必须阅读的文件、阅读目的和当前状态”，具体方案以同目录的 `GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md` 为准。

## 一、恢复阅读顺序

### 1. 实施方案主文档（必须完整阅读）

```text
/root/projects/codewhale-proxy/source/GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md
```

阅读目的：

- 了解最终目标和边界；
- 确认 gpt-5.6 使用 `/v1/responses`；
- 确认 DeepSeek/GLM/Kimi 保持 Chat 路径；
- 了解 TDD、旁路测试、缓存验收和回滚要求；
- 不要根据旧对话重新设计架构。

### 2. 项目 Git 状态

```bash
cd /root/projects/codewhale-proxy/source
git status --short --branch
git remote -v
git log --oneline --decorate -5
git fetch origin
git log origin/master --oneline -5
```

当前已知状态：

- 当前分支：`master`；
- 当前 HEAD：`12dee7e`；
- 工作树中已有未跟踪方案文档：

```text
GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md
```

- 目前没有 cc-proxy 源码改动；
- 新会话必须先创建新分支，不要直接在 master 开发。

建议分支名：

```text
feat/gpt-responses-transport
```

### 3. 项目入口与依赖

```text
/root/projects/codewhale-proxy/source/Cargo.toml
/root/projects/codewhale-proxy/source/Cargo.lock
/root/projects/codewhale-proxy/source/README.md
```

阅读目的：

- 确认 Rust edition 和依赖；
- 确认 axum、reqwest、serde、tokio、SSE 相关实现方式；
- 遵循现有项目结构，不随意引入新依赖。

### 4. 配置文件

```text
/root/projects/codewhale-proxy/source/config.toml
```

阅读目的：

- 确认 `[models]` 和 `[models.mapping]`；
- 确认已有 `gpt-5.6-luna` provider/profile；
- 确认当前 `claude-sonnet-4-6` 映射；
- 增加 `wire_api = "responses"` 时避免误改其他 profile。

已知重要配置事实：

```toml
"claude-sonnet-4-6" = "gpt-5.6-luna"
```

已有 gpt profile，但需要新增：

```toml
wire_api = "responses"
```

### 5. 配置加载和模型路由

```text
/root/projects/codewhale-proxy/source/src/config.rs
/root/projects/codewhale-proxy/source/src/routes/messages.rs
/root/projects/codewhale-proxy/source/src/routes/mod.rs
```

阅读目的：

- 理解 `Config`、`ModelProfile`、模型映射和 alias 查找；
- 找到最小的模型级 wire API 路由接入点；
- 确保只让 gpt-5.6 走 Responses；
- 确保 DeepSeek/GLM/Kimi 仍走当前 Chat 路径。

预期改造重点：

```text
ModelProfile 增加 wire_api
Config 增加 wire_api_for_model()
routes/messages.rs 按 upstream_model 的 wire_api 分支
```

### 6. 上游 HTTP 客户端

```text
/root/projects/codewhale-proxy/source/src/client.rs
```

阅读目的：

- 复用当前 reqwest client、timeout、连接池和认证 header；
- 增加 Responses 非流式和流式方法；
- 不改变现有 `chat_completion()` 和 `chat_completion_stream()` 行为。

预期新增方法：

```text
responses_completion()
responses_completion_stream()
```

上游路径必须为：

```text
/v1/responses
```

### 7. Anthropic 输入类型和现有 Chat 转换

```text
/root/projects/codewhale-proxy/source/src/anthropic/types.rs
/root/projects/codewhale-proxy/source/src/anthropic/converter.rs
/root/projects/codewhale-proxy/source/src/reasoning/build_messages.rs
/root/projects/codewhale-proxy/source/src/reasoning/relocate.rs
/root/projects/codewhale-proxy/source/src/reasoning/sanitize.rs
/root/projects/codewhale-proxy/source/src/reasoning/should_replay.rs
```

阅读目的：

- 了解 Anthropic Messages 的 system、message、tool_use、tool_result、thinking 结构；
- 复用模型映射和 system 前处理；
- 复用 billing nonce 稳定化和 volatile system block relocation；
- 避免为 Responses 复制一份会破坏 cache 的逻辑；
- 保持当前 Chat converter 的行为不变。

关键缓存约束：

```text
stabilize_metadata()
migrate_volatile_system_blocks()
CODEMERMAFROST_RELOCATE
```

必须保留并在 Responses 路径复用。

### 8. 现有 Chat 请求/响应类型

```text
/root/projects/codewhale-proxy/source/src/openai/types.rs
/root/projects/codewhale-proxy/source/src/openai/converter.rs
```

阅读目的：

- 了解现有 provider reasoning、tool call、usage 映射方式；
- 复用错误处理和 stop reason 的项目惯例；
- 不把 Responses 专用结构强行塞进 Chat 类型；
- 不修改 DeepSeek/GLM/Kimi 的现有响应转换行为。

### 9. 现有 Chat SSE 状态机

```text
/root/projects/codewhale-proxy/source/src/sse/mod.rs
/root/projects/codewhale-proxy/source/src/sse/stream.rs
```

阅读目的：

- 了解 Anthropic SSE 事件类型和当前状态机；
- 新增独立 `src/sse/responses.rs`；
- 不直接改变现有 `stream.rs` 的 Chat 行为；
- 复用 Anthropic SSE 输出类型，但不能复用 Chat 输入解析逻辑。

### 10. 现有 reasoning 和 cache fingerprint

```text
/root/projects/codewhale-proxy/source/src/reasoning/mod.rs
/root/projects/codewhale-proxy/source/src/reasoning/apply_effort.rs
/root/projects/codewhale-proxy/source/src/reasoning/prefix.rs
```

阅读目的：

- 理解当前 provider effort 映射；
- 确认 DeepSeek 的 `xhigh → max` 与 GPT 的 `xhigh → xhigh` 差异；
- 设计 Responses 专用 prefix fingerprint；
- 不能只 hash 工具名，必须包含稳定序列化后的 instructions 和 tools/schema。

### 11. 服务入口

```text
/root/projects/codewhale-proxy/source/src/main.rs
/root/projects/codewhale-proxy/source/src/routes/health.rs
```

阅读目的：

- 确认启动、配置加载、client 初始化和 health check；
- 确认新增 Responses 逻辑不需要修改服务生命周期；
- 当前阶段不部署生产服务。

## 二、必须参考的 GitHub 项目

### 主参考：Rust

```text
https://github.com/mxyhi/token_proxy
commit: 6bed3d1ebbbb44c06833d37b34b2ebe49cc8d8a2
license: Apache-2.0
```

重点文件：

```text
crates/token_proxy_runtime/src/proxy/anthropic_compat/request.rs
crates/token_proxy_runtime/src/proxy/anthropic_compat/response.rs
crates/token_proxy_runtime/src/proxy/anthropic_compat/tests.rs
crates/token_proxy_runtime/src/proxy/server/dispatch.rs
```

用途：参考 Rust 的 Anthropic↔Responses 字段映射、tool call 和路由。

### SSE/续接参考

```text
https://github.com/tangsipeng/openai-responses-anthropic-proxy
commit: cdba6dadd8625f27910cded16b22f6bd797d1aff
```

重点文件：

```text
src/translate.ts
src/server.ts
src/state.ts
src/server.test.ts
```

用途：参考 Responses SSE 事件状态机、tool continuation、`previous_response_id` fallback 和测试事件序列。

注意：本次未确认其许可证，不直接复制代码。

### codec/事件模型参考

```text
https://github.com/Lokesh-Chimakurthi/rosetta-llm
commit: 0c86c36ceeb414416f8b067a9f2b312f1fb85eab
```

重点文件：

```text
src/rosetta/codecs/openai_responses.py
src/rosetta/stream_codecs/openai_responses.py
src/rosetta/pipeline.py
```

用途：参考独立 codec、SSE 事件模型和 usage 处理。

### cache/provider 参数反例

```text
https://github.com/musistudio/claude-code-router/issues/1372
https://github.com/musistudio/claude-code-router/issues/1515
```

必须吸收：

- 动态 `x-anthropic-billing-header/cch` 会破坏 Responses cache prefix；
- 不支持的 `thinking`/`reasoning` 字段不能无条件转发。

## 三、必须恢复的已验证事实

### 上游能力矩阵

```text
gpt-5.6-luna + Chat tools                         = 200
gpt-5.6-luna + Chat reasoning_effort=xhigh       = 200
gpt-5.6-luna + Chat tools + reasoning_effort     = 400
gpt-5.6-luna + Responses tools + max             = 200
 gpt-5.6-luna + Responses tools + xhigh            = 200
```

### Responses tool-call 事件

```text
response.created
response.in_progress
response.output_item.added
response.function_call_arguments.delta
response.function_call_arguments.done
response.output_item.done
response.output_text.delta
response.output_text.done
response.completed
```

### Responses cache

稳定长前缀实际观察过：

```text
input_tokens=4073
预热后 cached_tokens=4070
命中率约 99.9%
```

短请求或首请求的 `cached_tokens=0` 不能作为无缓存证据。

## 四、当前工作边界

本次会话之后的开发必须遵守：

```text
只修改新分支
不修改生产服务
先单测，再旁路集成测试，再 Claude Code E2E
每个代码切片严格 RED → GREEN → REFACTOR
DeepSeek/GLM/Kimi 现有 Chat 路径必须回归
gpt Responses 失败不降级 Chat
```

## 五、第一轮命令清单

新会话从项目目录开始：

```bash
cd /root/projects/codewhale-proxy/source
sed -n '1,260p' GPT-5.6-RESPONSES-IMPLEMENTATION-PLAN.md
sed -n '1,240p' src/config.rs
sed -n '1,220p' src/client.rs
sed -n '1,220p' src/routes/messages.rs
git status --short --branch
git remote -v
git fetch origin
git log origin/master --oneline -5
git checkout -b feat/gpt-responses-transport
cargo fmt --check
cargo test
```

然后先做：

```text
WireApi 配置失败测试
```

不要先写 Responses 生产代码。

## 六、最终验收摘要

必须同时满足：

- gpt-5.6 实际走 eswitch `/v1/responses`；
- `reasoning.effort=max` 实际为 `reasoning.effort=max`；
- Function tools 非流式和流式均成功；
- tool result 至少两轮续接成功；
- Responses usage 能记录 cached_tokens/cache_write_tokens；
- 长稳定前缀第 3/4 次命中率至少 90%；
- DeepSeek/GLM/Kimi 仍走 Chat；
- 现有测试和 clippy 全通过；
- 旁路 E2E 通过后才允许讨论生产部署。
