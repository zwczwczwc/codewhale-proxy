# Kimi K3 Cache Optimization — Implementation Plan

> **文档性质：仅实施计划文档。本文件未修改任何 Rust 源码、未修改任何生产配置、未部署。**
> 仅含规划性/结构性内容；不含任何密钥、Authorization 头、完整 prompt、完整 tool schema、完整 reasoning 内容。
> 仓库基线：`origin/master` = `31f9b851308d2845b69d35880e35e1805b8e4f18`（PR #4 squash 合并后）。
> Feature 分支：`feat/kimi-k3-cache-optimization`（基于上述基线，已创建，未提交任何内容）。
> 关联报告：`/tmp/shared/kimi-cache-hit-issue/deepseek-reports/04*`（直连探针）、`06-generic-cache-architecture-recheck.md`（架构）、`07-leader-evidence-review-deepseek-v4-flash.md`（复核/门控）。

---

## 0. 状态

- 本计划是 Leader 授权下的**优化准备产物**：分支已建、文档已写；**未做任何实现、未 commit/push、未部署**。
- 任何后续实现必须重新经 Leader 授权，且逐 Phase 独立 commit、挂 golden 字节一致闸。
- `prompt_cache_key` 的 session key contract（G4）**未定**，实现前必须由路由层定义 per-session 稳定 key（重连不换 key）；**禁止凭空生成/每请求新 UUID**。

## 1. 目标 / 非目标

### 目标
- 在不改变生产行为（wire/SSE/usage 字节不变）的前提下，把 Kimi K3 链路的**缓存命中率**提升到可观测、可验收的水平，并把缓存相关逻辑收敛为声明式 policy + 统一 telemetry。
- 建立与 04 探针受控结论一致的**可测验收基线**（固定 model/effort、byte 稳定前缀、完整 assistant 回放、prompt > 阈值、同后端）。
- 为「生产 0% 命中根因」的最终判定（G2 outbound 抓包）提供实现侧的可验证假设。

### 非目标
- 不强行把 Responses wire 统一为 Chat（或反之）——两套 encoder 的词汇/文法/usage 字段语义保留 provider/wire-specific。
- 不做「为统一遥测而抹平原始字段语义」：cache creation（prompt−cached 推断）与上游重写 cached_tokens 分开存储，保证验收可区分 creation/read/miss。
- 不改路由层多后端选择策略本身（eswitch 多后端路由根因尚未实证）。
- 不新增 `if provider == "moonshot"` / `starts_with("kimi")` 硬编码——能力一律走 capability/policy 数据。

## 2. 约束（Kimi 官方 + 直连探针）

来源：04 直连探针（HIGH）、07 复核（生产根因 CONDITIONAL）。不复制任何原始请求内容，仅列结论性约束。

- **命中最小充分条件（受控实验支持，探针内 HIGH）**：
  - 固定 model / effort 组合；
  - 公共前缀 byte 稳定（system + 工具定义稳定序列化，历史 append-only）；
  - 完整 assistant 轮回放（reasoning + text + tool_calls 完整输出，独立于当前请求 thinking）；
  - 请求体 prompt 长度 > 阈值（探针中 >256 tokens 级）；
  - 同一上游后端（跨后端 = 缓存池隔离 → miss）。
- **负结果**：`prompt_cache_key` 不是命中必要条件（同一 wire、仅改 key 的后端 100% 命中即证）。
- **Kimi effort 枚举**：`low / high / max`（官方）；**切换 effort 会破缓存** → `effort_pin_per_session` 属行为变更，仅允许在 Phase 3/4 且非 moonshot 行为不变时启用。
- **生产 0% 根因 = eswitch 多后端路由**：目前是 **CONDITIONAL（强假设）**，需 G2 生产 outbound 抓包对照后才能升格为已证根因。

## 3. 当前 Chat / Responses 架构（事实基线，来源 06 §1，行号均已复核）

- 单一入口 `routes/messages.rs::handle_messages`（L72-242）按 `wire_api_for_model`（L87）分流，同源 `anthropic::types::MessagesRequest`。
- 共用 `map_model_to_upstream`（anthropic/converter.rs L20-31）、`ProviderConfig`（config.rs L25-49）、relocate 生态（relocate.rs L98-123）、工具按名排序（Chat converter.rs L118-120 / Responses request.rs L52-56 各自实现）。
- **重复（同语义两套实现，无共享 IR）**：
  - system→text：`system_prompt_to_text`（build_messages.rs L140-155） vs `system_text`（request.rs L188-199）；
  - 消息→wire：`build_chat_messages_with_reasoning`（build_messages.rs L44-138） vs `append_message`（request.rs L201-246）；
  - 工具转换：converter.rs L263-282 vs request.rs L272-287；
  - tool_result→text：build_messages.rs L240-257 vs request.rs L256-270；
  - cache stats 算术三处重复：openai/converter.rs L64-87 + `SseStateMachine::finalize` L389-412 + responses/response.rs `cache_stats_from_usage` L40-53 + stream.rs L15-61。
- **缓存遥测字段各异（统一 telemetry 的目标）**：
  - Chat 只读 `prompt_tokens_details.cached_tokens`（openai/types.rs L207-210）；
  - DeepSeek 顶层 `prompt_cache_hit/miss_tokens` 已定义（L222-229）但未用于映射；
  - **Kimi 顶层 `usage.cached_tokens` 未读**（定义了没接）；
  - Responses 读 `input_tokens_details.cached_tokens/cache_write_tokens`（response.rs L147-155）。
- **明确不抽象（保持 provider/wire-specific）**：
  - Responses item 词汇（function_call/function_call_output/input_text/output_text/reasoning）；
  - Kimi `reasoning_effort` 枚举 + 不用 K2 thinking（capability 数据）；
  - 三套 SSE 事件文法（Chat delta / Responses response.*.* / Anthropic 输出）；
  - `message_start.usage` 必须对象不能 null（responses/stream.rs L550-558，Claude Code Agent 崩溃点）。

## 4. 阶段计划（文件级）

### Phase 0 — 前置（已完成）
- moonshot-official 路由 + `message_start.usage` 对象化已通过 PR #4 squash 合并至远程 master（`31f9b851`）。`cargo test --locked --all-targets` = 101 passed / exit 0。
- 本分支即基于该基线创建。

### Phase 1 — Conversation IR（纯重构，零行为变化）
- 新增 `src/conversation.rs`（+`mod` 注册）；`reasoning/build_messages.rs`、`responses/request.rs` 内部改走 IR。
- 规范化（角色分发、thinking→reasoning、tool result 扁平、system 拆分）从两个 encoder 提出，wire 阶段不碰历史。
- **NO-GO 闸**：同一 fixture 新旧 wire 字节不一致 → 整体 NO-GO。

### Phase 2 — capability policy + deterministic schema + cache 边界
- `src/config.rs`：`ProviderConfig` 增 `cache_policy` 块（声明式：key_field/key_source、effort_enum、effort_pin_per_session、replay、history、tool_schema、usage）。
- 新增 `src/schema.rs`：canonical serializer（tools `parameters/description` 键排序序列化，跨轮 byte-stable）。
- 新增 `src/cache.rs`：统一 `CachePolicy` + `CacheStats` telemetry，替换三处重复算术；`prompt_cache_key` 注入由该层按 session key 契约执行。
- **NO-GO 闸**：未 opt-in provider 的 tools 字节变化。

### Phase 3 — Kimi policy 激活（config-only + 少量 plumbing）
- `config.toml` `[providers.moonshot]` 增 `cache_policy`；路由层传 per-session key；同 session 多轮对比 hit rate。
- **NO-GO 闸**：为支持 Kimi 新增任何 `if provider=="moonshot"` / `starts_with` 硬编码——policy 必须是唯一通道。

### Phase 4 — replay / history gate
- `append_only` gate 掉 `cleanup_orphan_tool_calls`（build_messages.rs L135/L357-447）、`compact_tool_result`（L14-39）、tool_result 去重（L90-96）；Chat relocate 改走 split+合成尾部（request.rs L139-154）。
- `full_assistant` replay 独立于当前 effort（替换 converter.rs L43-60 用当前 effort 重算 include_reasoning；禁用占位符 build_messages.rs L323-329、sanitize.rs L39-41）。
- **NO-GO 闸**：非 moonshot provider 行为变化。

### Phase 5 — 回归与 parity
- 统一 cache stats 后全量回归；确认 GPT Responses、DeepSeek/GLM Chat 不受影响。
- **NO-GO 闸**：任一既有 provider 的 wire/SSE/usage 输出变化。

## 5. 测试矩阵

- 主路径：Chat 非流/流 × Responses 非流/流。
- 内容：text-only、thinking+text、tool_call→tool_result 续写、多轮 replay、redacted thinking。
- cache usage 字段：top-level `cached_tokens` vs `prompt_tokens_details` vs `input_tokens_details` vs `prompt_cache_hit/miss`；hit_rate/cache_creation 算术。
- 错误/EOF：`response.failed/error/cancelled`（stream.rs L491-524）、EOF 无 terminal（L684-695）、`message_start.usage` 对象契约（L550-558）。
- 确定性：canonical schema 跨轮 byte-stable、工具顺序、指纹相等（prefix.rs L91-133 已测顺序无关，补 schema 内容级）。
- golden：IR 迁移后 Chat↔Responses 同 fixture 字节一致（**先行测试，不绿则整体 NO-GO**）。

## 6. NO-GO / 回滚门

- **G1（阻塞全部）**：dirty 工作树未固化——已由 PR #4 解决（Phase 0 合并）。
- **G2（阻塞根因判定）**：生产 outbound 抓包对照（cc-proxy→clawbot outbound body 前缀 hash + cached 配对 + 后端切换时间线）。未做前生产 0% 根因维持 CONDITIONAL，不得据此改路由。
- **G3（阻塞 Phase 3 激活）**：读回 live `/etc/cc-proxy/config.toml` 确认 effort_map（官方枚举）与 moonshot provider 设置。
- **G4（设计契约）**：session 稳定 key 定义（路由层 per-session 稳定 `prompt_cache_key`，重连不换 key）。**未定前禁止注入 prompt_cache_key。**
- **G5（技术 NO-GO 闸）**：Phase 1 golden 字节一致不绿 → 整体 NO-GO。
- 回滚：每 Phase 独立 commit；NO-GO → revert 该 Phase、不发布。线上回滚 = 从 master 重部署二进制。

## 7. 生产边界（本任务全程遵守）

- 未改 `/etc/cc-proxy`、未改 systemd、未重启/部署、未触碰 `/usr/local/bin/cc-proxy`、未触碰 11441、未调用任何 Kimi/cc-proxy 业务 API。
- 未 commit/push/merge；未 reset/clean；未触碰 stash `stash@{0}`（stale WIP on reform/cc-proxy-kimi-k3）。
- 未跟踪 Markdown/tools（29 项）不得进入任何提交。
