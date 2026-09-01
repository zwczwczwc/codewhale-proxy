# Kimi K3 Cache Optimization — FINAL Implementation Plan（最终开发方案，可压缩上下文后直接执行）

> **requested_model=deepseek-v4-flash**
> provider=`eswitch`（api=`http://100.64.0.1:11434/v1`，读自 `~/.hermes/config.yaml` providers.eswitch）
> api_mode=`chat`（平台/会话上下文声明；与 04/07/08/10/18/20/21/22 同源）
> **MODEL_IDENTITY_UNVERIFIED** —— 沙箱内无法从运行时独立证明底层模型；仅如实记录 harness 声明与 requested_model，不冒充。
>
> **文档性质：仅最终实施计划 + 上下文恢复文档。本文件及配套 CONTEXT-RECOVERY 未修改任何 Rust 源码、未修改任何生产配置、未部署、未 commit/push。**
> 不含任何密钥 / Authorization 头 / 完整 prompt / 完整 tool schema / 完整 reasoning 原文（仅哈希/长度/结论）。
>
> **本方案由以下报告整合而成（均已读毕）**：20（事实矩阵）、21（架构审查）、22（测试矩阵审查）、18（差异裁决）、19（单一来源治理）、08（Phase 0 落地）、10（feature 基线）、04（直连探针复核）、07（门控）、06（架构初稿），以及 `results.json`/`probe.py` 与当前源码逐行复核。
> 它是 `KIMI-K3-CACHE-OPTIMIZATION-PLAN.md`（旧版）的**修订与终版**，旧版中的下述「错误/不可成立」条目以本文件为准。

---

## 0. 执行摘要（TL;DR）

1. **方向维持 CONDITIONAL GO + 门控体系**（07 五门控 G1–G5 + 21/22 增补 G6/G7 与发布阻断 B1–B10）。未获证据不推进。
2. **相对旧 PLAN 的五处必须修正**（21/22 裁决，实现前必读）：
   - ① `schema.rs` 从「canonical serializer（wire 变换）」**降级为「telemetry 指纹 + 稳定性验证（断言，不变换）」**——`serde_json` 1.0.150 未启用 `preserve_order`（`Cargo.lock` grep=0），`Map` 为 BTreeMap，对象键重序列化已按字母序稳定；真正破坏前缀稳定的是**行为变换**（cleanup/compact/去重/占位符/include_reasoning 随当前 effort 翻转），这些才是 Phase 4 目标。
   - ② **不做「单一 generic encoder」**：最小可行 = 一个 lean `ConversationIR`（只承载两 encoder 公共的规范化决策）+ **两个保持独立的 wire encoder 函数**。
   - ③ **Phase 4 拆 4 个独立子提交**（effort pin → full_assistant replay → append_only → relocate tail 切换），每子步单独测 hit-rate、单独回滚，保证命中率变化单变量归因。**`prompt_cache_key` 注入不在 Phase 4**——它在 **Phase 3**（fail-closed，§3.3/G4）；Phase 4 四子步只做行为 gate。
   - ④ **事实修正**：`openai/types.rs::Usage`（L217-230）**根本没有顶层 `cached_tokens` 字段**（是「未定义」而非「定义了没接」）——落地须**新增** `#[serde(default)] cached_tokens: Option<u32>`，再经 policy `usage="top_level_cached_tokens"` 映射进统一 telemetry。
   - ⑤ **provider 名三处不一致**：内置默认 `fireworks`（config.rs L454/L510）、仓库 config.toml `moonshot`（L59、profile L115）、路由特判 `moonshot-official`（routes/messages.rs L67）→ Phase 3 必须经 policy `upstream` 绑定统一为**单一 canonical 名**；否则官方客户端对仓库自身配置是**死路由**。
3. **golden 门语义修正**（22 G1）：**删除「IR 迁移后 Chat↔Responses 同 fixture 字节一致」**（物理不可实现：词汇/文法不同）。改为：
   - golden-1（MUST）：**同 wire 旧编码器 vs 新 IR 编码器，字节级相等**（逐 Phase fixture 捕获）；
   - golden-2（SHOULD）：同 IR 前提下 Chat 与 Responses **语义 parity**（内容序列/角色/工具往返，非字节）；
   - golden-3（MUST）：同一输入重复编码字节相等（确定性）。
4. **04 探针的后端路由结论只能是 conditional**：探针内 C2a(0%) vs C2b(100%) 为干净受控对照（HIGH），但「eswitch 多后端路由机制」仅由返回 `model`/`provider` 标签**推断**，未直接观测路由内部；且 R2/R4 同标签 0%/95% → 上游内部策略仍含未知（U4）。**生产 0% 根因 = eswitch 多后端路由/缓存池隔离 是 CONDITIONAL 强假设**，须 G2（生产 outbound 抓包 + 前缀 hash + cached 配对 + 后端切换时间线）升格；升格前**不得**据此改路由/改 cc-proxy 之外系统。
5. **`prompt_cache_key` 是可选增强，非命中必要条件**（C2b 仅改 key 100% 命中，负结果 HIGH）；**无稳定 session source 必须 fail-closed**（不注入字段，绝不用随机 UUID 兜底）。
6. **真实验证隔离**：staging cc-proxy 实例用独立端口 **11449**（`LISTEN_ADDR=0.0.0.0:11449` + 独立 config），**生产 11441 全程不触碰**，直到 Leader 显式授权发布。

---

## 1. 目标 / 非目标

### 1.1 目标
- 在不改变**未 opt-in provider**（deepseek/glm/gpt Responses）生产行为（wire/SSE/usage 字节不变）的前提下，把 Kimi K3 链路缓存命中率提升到**可观测、可验收**水平，并把缓存相关逻辑收敛为声明式 policy + 统一 telemetry。
- 建立与 04 探针受控结论一致的**可测验收基线**（固定 model/effort、byte 稳定前缀、完整 assistant 回放、prompt>256、同后端）。
- 为「生产 0% 根因」最终判定（G2）提供实现侧可验证假设，并给出可单变量归因的改造路径。

### 1.2 非目标（不得做）
- ❌ 不强行统一 Chat/Responses wire（词汇/文法/usage 字段语义保留 provider/wire-specific）。
- ❌ 不做「单一 generic encoder」把两种 wire 折叠进一个函数（21 §1）。
- ❌ 不做「为统一遥测抹平原始字段语义」：creation（仅上游显式报 write 才填）/read/miss 分桶，**Kimi 未报 write 时 creation=None，不得用 `prompt−cached` 伪造**。
- ❌ 不把 schema canonicalization 实现成**无必要的 wire 重排**（serde_json 已保证键序稳定；只做验证/指纹，不变换 wire 字节）。
- ❌ 不改路由层多后端选择策略本身（G2 前）；不新增 `if provider=="moonshot"`/`starts_with("kimi")` 硬编码——能力一律走 policy 数据。
- ❌ 不把参考项目（raine/musistudio/m0n0x41d）当作现成完整 cache 方案直接复制（仅借鉴 key 接线、K3 分支、测试夹具思路）。

---

## 2. 已验证事实 / 假设 / 未知（证据分级：HIGH / MEDIUM / CONDITIONAL / UNKNOWN）

### 2.1 FACT（HIGH，独立可复现/直接实测）

| 编号 | 事实 |
|---|---|
| F1 | 生产命中观测（handoff §1.1）：Kimi 40 样本 token 加权 50.73%、p50 0%、**60% 请求完全 0%**；OpenAI Responses 523 样本 93.73% 正常 → 框架层无问题，问题在 Anthropic→Kimi 转换层或更上游。 |
| F2 | 链路：Claude Code → cc-proxy(:11441, systemd cc-proxy.service, PID 3731135) → eswitch(clawbot.hermes.tailnet, :11434) → Kimi K3。Kimi K3 自动前缀缓存，无 `cache_control`；>256 prompt tokens；官方示例 99.8%（相同请求重复）。 |
| F3 | 受控探针（9/9 HTTP 200，0 错误/超时）：R1 cold creation(cached=0 正常非失败)、R2 miss(跨后端)、R3 92.7% read-hit、R4 95.0% read-hit、C1(仅改 key) 93.4%、C2a(=R1 byte-identical wire 同 key)→0%、C2b(=R1 wire 仅改 key)→100%。**`prompt_cache_key` 非命中必要条件（负结果）**。prompt 增量 +161/+101/+101（append-only 正确性佐证）。关键 hash：R1 body `2eca58e7…`、C2b body `5f89648b…`；R2/R4 无 reasoning 时 sha256=`e3b0c442…`（空串）。 |
| F4 | 最小充分请求构造（探针内 HIGH；生产适用性 CONDITIONAL）：固定 `model=kimi-k3`；固定合法 effort（`reasoning_effort=max`，会话内不切换）；system/tools/messages 前缀 byte 级稳定（system 固定、tools 按 name 排序、history append-only）；上一 assistant 消息**完整回放**；prompt>256 tokens；**命中还需落在同一上游缓存池**。 |
| F5 | Kimi 官方（leader-synthesis §2）：自动缓存、无 cache ID/TTL、>256 token、固定 context 放前部追加尾部；`prompt_cache_key` 是 session/task 标识（Kimi Code Plan 强建议，**非普遍硬性必填**）；K3 总是推理、顶层 `reasoning_effort`∈{low,high,max}、**不要用 K2.x `thinking`**、多轮必须完整回放 assistant message（含 reasoning_content/tool_calls）、**切换 effort 破缓存**。「byte-stable / canonical JSON」无官方明文 = 合理工程推断（MEDIUM）。 |
| F6 | 源码级缺口（存在性 HIGH，live 行为 UNKNOWN，行号均已复核）：无 `prompt_cache_key` 请求字段（openai/types.rs L4-28）；`Usage` 无顶层 `cached_tokens`（L217-230）；K3 effort 默认 `xhigh`（converter.rs L134-153），config.toml effort_map 含 `medium→medium`（L64）非法；reasoning 历史可被改写（include_reasoning 由当前 effort 决定 converter.rs L43-60；占位符 build_messages.rs L322-329、sanitize.rs L39-41；orphan cleanup L354-446）；tools 无完整 canonicalization + prefix 指纹只哈希工具名（prefix.rs L27-32）；Chat 只读 `prompt_tokens_details.cached_tokens`（openai/types.rs L207-210）；cache stats 算术四处重复（openai/converter.rs L64-87、SseStateMachine::finalize L389-412、responses/response.rs L14-53、responses/stream.rs L15-61）；双 encoder 重复无共享 IR。 |
| F7 | provenance（18 裁决 + 复核）：live `/usr/local/bin/cc-proxy` sha256=`f55cd98d…` == source/target/release（nlink=2 twin）；≠ master 树 `31f9b851` 重建 `2d0bfb5e…`（差 120B，纯 metadata：`.llvm.N` 计数器位数 + BuildID + e_shoff）；`.text/.rodata/.data/重定位/程序头` 逐字节相同、反汇编 0 差异 → **机器码/数据层功能等价**；build→commit 溯源**不可证**。 |
| F8 | 仓库状态：`origin/master=31f9b851`（PR #4）；`feat/kimi-k3-cache-optimization @ 31f9b851` 跟踪 origin/master 未 commit/push；HEAD 在临时分支 `chore/land-existing-cc-proxy-fixes @ 58f006b`（tree==master，上游已删）；untracked=31（30 .md + tools/），必须保留不入 commit；live config `/etc/cc-proxy/config.toml` root:root 600 不可读。 |

### 2.2 假设（ASSUMPTION）

| # | 假设 | 证据 | 等级 |
|---|---|---|---|
| A1 | **生产 0% 根因 = eswitch 多后端路由/缓存池隔离** | 探针 C2a(0%)/C2b(100%) 受控对照 HIGH；但路由机制仅由返回标签推断，R2/R4 同标签 0%/95% 未解释 | **CONDITIONAL（强假设）**，需 G2 升格 |
| A2 | cc-proxy 转换层自身在改写 wire（orphan cleanup/compact/redaction） | 源码路径存在（F6），生产 outbound 未抓 | CONDITIONAL/候选 |
| A3 | 生产 tools schema 跨轮漂移 | 代码无 canonicalization → 无法证明稳定 | CONDITIONAL |
| A4 | 生产 reasoning_content 被丢失/改写 | 源码条件性路径存在；live UNKNOWN | CONDITIONAL/候选 |
| A5 | 「byte-stable / canonical JSON」为命中必要条件 | 官方无明文；受控探针支持 | MEDIUM |

### 2.3 未知（UNKNOWN，取得证据才推进）

| # | 未知项 | 阻塞 | 取得方式 |
|---|---|---|---|
| U1 | **live `/etc/cc-proxy/config.toml` 内容**（effort_map 是否 low/high/max、moonshot provider 设置、reasoning_field、thinking） | Phase 3 激活 + 生产 effort 判定 | **G3**：root 受控只读读回脱敏值 |
| U2 | **生产 outbound Kimi body**（prompt_cache_key 有无/稳定、effort 实际值、reasoning 完整度、tools 字节） | 根因判定 A1/A2/A3/A4 | **G2**：cc-proxy→clawbot:11434 抓包 + 前缀 hash + cached 配对 + 后端切换时间线 |
| U3 | 生产 0% 与 eswitch 后端切换时间线相关性 | A1 升格 | G2 时间线 |
| U4 | eswitch/Kimi 上游内部缓存池/TTL 策略 | R2/R4 同标签不同结果 | 上游观测（超出只读范围） |
| U5 | 生产 reasoning_content 是否保留 | A4 | G2 body 分析 |
| U6 | `prompt_cache_key` 精确上游语义（顶层 vs 嵌套、计费字段） | 仅文档依据 | 文档/上游 |
| U7 | 构建确定性跨环境 | 发布溯源 | 新环境重建 |
| U8 | live vs master 重建行为级等价 | 未测 | 部署后 11441 行为回归（授权后） |
| U9 | GitHub 参考项目 Issues/PR | 参考价值边界 | 未读 |
| U10 | runtime 模型身份 | 无法独立证明 | Leader 用运行元数据复核 |

---

## 3. Kimi K3 Chat 最小请求契约（实现必须满足）

### 3.1 契约要素（受控探针 HIGH 支持；生产适用性 CONDITIONAL）
1. `model=kimi-k3` 固定，会话内不切换；
2. `reasoning_effort` ∈ 官方合法枚举 `{low, high, max}` 固定，会话内不切换（切换破缓存）；
3. system / tools / messages 前缀 **byte 级稳定**：system 固定；tools 按 `function.name` 排序且 schema 内容跨轮稳定；history **append-only**；
4. 上一 assistant 消息**完整回放**（content + reasoning + refusal + tool_calls 原样放回，独立于当前请求 thinking）；
5. 稳定 `prompt_cache_key`（官方建议；**实测非命中必要条件**，保持稳定无害）；
6. prompt > 256 tokens（探针 R1=6640，远超阈值）；
7. **命中还需落在同一上游缓存池**（eswitch 路由决定——R2/C2a 0% 探针内根因，生产根因 CONDITIONAL）。

### 3.2 cache usage 分类（四桶独立计数，验收必须能区分）
| 桶 | 定义 | 探针对应 | 备注 |
|---|---|---|---|
| cache creation（冷写） | 上游显式报 write 才计；未报 → None | R1（cached=0，首轮建前缀，正常非失败） | Chat 侧现用 `prompt−cached` 推断会把「miss 且 cached=0」误记 creation → **禁止** |
| cache read-hit | 读命中 cached>0 | R3/R4/C1/C2b | Kimi 顶层 `usage.cached_tokens`（新增字段）或 eswitch 重写的 `prompt_tokens_details.cached_tokens` |
| cache miss | cached=0 且非 creation（同池内未见缓存） | R2/C2a（跨后端） | 与 HTTP error 不混淆 |
| HTTP error/timeout | 非 200 / 超时 | 探针 0/9 | 独立桶 |

### 3.3 `prompt_cache_key` 契约（G4）
- **定位**：可选增强（官方建议、Kimi Code Plan 要求/强建议），**非命中必要条件**（C2b 负结果）；不能替代前缀稳定性。
- **fail-closed 规则**：无稳定 inbound session 标识 → **不注入 `prompt_cache_key` 字段**（省略字段，沿 `request_id #[serde(skip)]` 先例 responses/types.rs L7-8）；记录 `cache_key_source=none` 供监控。**绝不允许 `Uuid::new_v4()` 兜底**（每请求新 UUID 必然破缓存）。
- **session key 来源优先级**（21 §3.2 可执行契约）：
  1. `metadata.user_id`（Claude Code 每次请求带稳定 per-install user id）——converter 内可取，最低成本；
  2. 可选入站 header（如 `x-cc-session-id`）由 cc-connect/ingress 设置 → `handle_messages` 用 Axum extractor 取（后续扩展）；
  3. 以上皆无 → fail-closed。
- **key 定义**：`session_key := sha256( upstream_provider | model | source_name | source_value )[..16]`；只随 (user_id/session, model, provider) 稳定；跨后端天然隔离（缓存池 per-backend）；进程重启不换 key（入站信号的确定性哈希，无状态）。
- **注入点（最小改动）**：`cache.rs::session_key(req, policy, model, provider) -> Option<String>`，在两个 encoder 内调用写入 request 结构体字段；**client.rs 不需要动**；key 只存在于 outbound request，不进 Anthropic 响应/SSE。

---

## 4. 当前代码问题清单（confirmed / live-unknown / upstream-hypothesis）

### 4.1 Confirmed source risk（源码存在性 HIGH，live 行为 UNKNOWN）
| # | 风险 | 位置（复核行号） | 处理阶段 |
|---|---|---|---|
| C1 | Chat 请求无 `prompt_cache_key` 字段 | openai/types.rs L4-28 | Phase 2/3 |
| C2 | `Usage` 无顶层 `cached_tokens` 字段（serde 静默丢弃） | openai/types.rs L217-230 | Phase 2 |
| C3 | K3 effort 默认 `xhigh`，config.toml effort_map 含 `medium`（非官方枚举）→ 上游 400 风险 | converter.rs L134-153；config.toml L64 | Phase 3（G3 后） |
| C4 | reasoning 历史可被改写：include_reasoning 由当前请求 effort 门控（注释 L312-321 与实现 L43-60 有缝隙）；占位符注入 | converter.rs L43-60；build_messages.rs L312-345；sanitize.rs L39-41 | Phase 4（full_assistant） |
| C5 | 无条件行为变换破坏前缀字节稳定：orphan cleanup / compact_tool_result / tool_result 去重 | build_messages.rs L14-39/L87/L90-96/L135/L357-447 | Phase 4（append_only） |
| C6 | tools 无完整 schema canonicalization；prefix.rs 指纹只哈希工具名（检测不到 schema 内容漂移） | prefix.rs L27-32 | Phase 2（升级指纹为完整前缀哈希，仅验证不变换） |
| C7 | Chat 遥测只读 `prompt_tokens_details.cached_tokens`，不读 Kimi 顶层 cached_tokens；DeepSeek `prompt_cache_hit/miss` 已定义未接（`#[expect(dead_code)]`） | openai/types.rs L207-210/L222-229 | Phase 2 |
| C8 | cache stats 算术四处重复、语义略异（Chat creation=prompt−cached 混淆 miss；Responses 三桶分离） | openai/converter.rs L64-87；sse/stream.rs L389-412；responses/response.rs L14-53；responses/stream.rs L15-61 | Phase 2 |
| C9 | 双 encoder 重复无共享 IR（system→text、消息→wire、tool_result→text、工具转换各两套） | build_messages.rs L44-138/L140-155/L240-257/L263-282 vs responses/request.rs L201-246/L188-199/L256-270/L272-287 | Phase 1 |
| C10 | Chat relocate 走改写式 `migrate_volatile_system_blocks`（追加进末消息且不查 role）；Responses 走 split+合成尾部 | relocate.rs L125-199 vs L212-231；request.rs L139-154 | Phase 4（4b） |
| C11 | **provider 名三处不一致**：内置 `fireworks`（config.rs L454/L510）vs 仓库 config.toml `moonshot`（L59/L115）vs 路由特判 `moonshot-official`（routes/messages.rs L67）→ 官方客户端对仓库自身配置是死路由 | config.rs / config.toml / routes/messages.rs | Phase 3（upstream 绑定统一） |
| C12 | `effort_map` 校验缺失：映射结果 ∈ effort_enum 未在 `validate()` 校验（silent-fail 原则） | config.rs L174-286 | Phase 2/3 |

### 4.2 Live unknown（阻塞判定，须 G2/G3 取证）
- U1 live config 内容；U2 生产 outbound body（key 有无/effort 值/reasoning 完整度/tools 字节）；U5 reasoning_content 是否保留；U3/U4 后端切换时间线。

### 4.3 Upstream hypothesis（CONDITIONAL，未证不得据此行动）
- A1 eswitch 多后端路由/缓存池隔离（生产 0% 根因候选）；A4 reasoning 丢失；A3 tools 跨轮漂移；A2 转换层改写 wire；U4 上游内部策略。

---

## 5. 最小架构

### 5.1 采纳的抽象（21 §1 逐项判定）
| 抽象 | 判定 | 范围 |
|---|---|---|
| **A. Conversation IR** | ✅ 采纳，**收敛范围** | 只承载两 encoder 公共的**规范化决策**，不建模 wire 词汇。IR 归一化：system 拆分（stable/volatile）、thinking→reasoning、tool_result 扁平、role 分发、显式 `synthetic_tail` 槽 + reasoning 字段（Chat 回放 reasoning_content；Responses 编码时丢弃 thinking——语义差异由 encoder 保留，IR 不得抹平）。 |
| **B. capability policy** | ✅ 采纳（最小、最高杠杆） | `ProviderConfig` 增 `cache_policy`；声明式取代 provider 特判；同时承载 `upstream` 绑定（替换 select_client 字符串匹配）与 `usage` 字段映射。 |
| **C. 两个 wire encoder** | ⚠️ **保持独立** | 保留 `convert_request`（Chat，anthropic/converter.rs L34-187）与 `convert_request`（Responses，responses/request.rs L18-113）两个独立函数，共享 IR + policy + 工具排序 helper + canonical hash helper。 |
| **D. schema.rs** | ❌ **重定位** | 从「wire 变换」降为「telemetry 指纹 + 稳定性验证（断言而非改写）」。`serde_json` 1.0.150 无 `preserve_order`（Cargo.lock grep=0）→ 键序已稳定。把 request.rs L115-137 `canonical_hash/canonicalize` 提为共享；升级 prefix.rs 指纹为完整 canonical 前缀哈希。 |
| **E. cache.rs** | ✅ 采纳（最高价值新模块） | `CachePolicy` + `CacheStats`（raw input/read/write 三 Option 分离）+ usage 映射表 + `session_key()`；统一 4 处重复算术，但**原始字段分离保留**（creation/read/miss 不混淆）。 |

### 5.2 明确拒绝的过度重构
- 统一 Chat/Responses wire 词汇；统一三套 SSE 事件文法；把 reasoning replay/orphan cleanup/占位符并入 IR（留在 policy-gated 行为 helper）；把 `get_reasoning` 替换为泛型框架；为统一遥测抹平原始 usage 语义。

### 5.3 共享 vs 独立（21 §2）
- **共享/统一**：inbound `MessagesRequest` 单入口、模型映射 `map_model_to_upstream`、capability policy（ProviderConfig）、工具按名排序（提为共享 helper）、canonical hash（提为共享）、cache stats/telemetry（统一进 cache.rs）、usage 字段映射。
- **必须独立**：wire 消息词汇（Chat role/tool/reasoning_content/tool_calls vs Responses input_text/output_text/function_call/function_call_output）；thinking 处理语义（Chat 回放 reasoning_content，Responses 丢弃 thinking）；三套 SSE 事件文法；`message_start.usage` 对象契约；effort 形状（Chat 顶层字符串 vs Responses 对象 reasoning.effort）；`request_id #[serde(skip)]` 先例。

---

## 6. 6 阶段实现计划（每 Phase 独立 commit；NO-GO→revert 该 Phase 不发布）

> 依赖标注：[Gx]=必须先满足的门控；[报告]=必读报告。每 Phase 完成后跑 `cargo test --locked --all-targets`（基线 101 passed）。

### Phase 0 — 前置（✅ 已完成，勿重做）
- PR #4 squash 合并至 `origin/master` `31f9b851`：moonshot-official 路由 + `message_start.usage` 对象化。`cargo test --locked --all-targets` = 101 passed / exit 0（08 报告）。
- feature 分支 `feat/kimi-k3-cache-optimization` 已创建 @ `31f9b851`（未 commit/push）。**工作基线 = origin/master `31f9b851`（tree `6e1a2132`）**。

### Phase 1 — Conversation IR（commit 1，零行为变化）[G5 先行]
- **显式边界：Phase 1 不改变 Chat relocate**。Phase 1 仅做 encoder 内部 IR 迁移，wire 字节必须逐字节通过 per-wire golden（T01/T02）。**Chat relocate 现状（`migrate_volatile_system_blocks`，relocate.rs L125-199）保持不变**；把 Chat relocate 切换到 split+合成尾部**只属于 Phase 4d**（`relocate` policy-gated，moonshot 仅）。范围漂移 → golden 不等 → NO-GO。
- **先捕获 golden，后重构（顺序关键）**：
  - 新增 `tests/golden/*.json`——用**当前** encoder 对固定 fixture 生成 wire 字节快照（Chat 非流/流 × Responses 非流/流 × relocate on/off），以 sha256 入库。fixture 集 = 4 主路径 × relocate 两态 × 内容（text/thinking/tool/多轮/refusal/image/null）。
  - **注**：`CODEMERMAFROST_RELOCATE` 是进程级 env var，capture 两态需串行/显式隔离，防并行测试 env 竞争（22 G1 golden-3）。
- **golden harness 结构（推荐方案，27 SF-1，Phase 1 委派书必须钉死）**：仓库为 **binary-only crate（无 `lib.rs`、无 `tests/` 目录）**，`tests/` 集成测试无法 `use cc_proxy::…` 链接内部符号——**不新增 `src/lib.rs`**。golden capture 逻辑与 `test_config()` 放相关模块的 **`#[cfg(test)]`**（可直接调用内部 encoder 符号）；`tests/golden/*.json` **仅作数据**，由 `CARGO_MANIFEST_DIR` 路径读取。零结构性改动，符合 Phase 1「零行为变化」。
- 新增 `src/conversation.rs`（+main.rs `mod` 注册）：lean IR = system(stable/volatile split) + turns(User/Assistant{reasoning,text,tool_calls}/ToolResult) + `synthetic_tail`；只做规范化，不承载 wire 词汇。
- `reasoning/build_messages.rs`（L44-138）与 `responses/request.rs`（L18-113）内部改走 IR；工具排序提为共享 helper（一行排序）。
- **顺带（测试确定性）**：把 responses/request.rs 测试里的 `Config::from_env()`（L300/L381/L423/L449/L466）改为固定测试 `test_config()` fixture（现依赖运行环境 config，潜在非确定性）。
- 文件：新增 `src/conversation.rs`、`tests/golden/*.json`（**仅数据**）；golden capture 与 `test_config()` 走模块内 **`#[cfg(test)]`**（不新增 `tests/common/mod.rs`、**不新增 `src/lib.rs`**）；改 `src/main.rs`(mod)、`src/reasoning/build_messages.rs`、`src/responses/request.rs`、`src/reasoning/mod.rs`(helper 注册)。
- 测试：golden-1（per-wire 旧 vs 新字节相等，MUST）、golden-3（重复编码确定性，MUST）、IR 单元测试。
- **NO-GO（G5）**：任一 golden 字节不一致 → **整体 NO-GO**。`cargo test --locked --all-targets` 保持 101 passed（+新增全绿）。
- 提交：`feat(ir): introduce lean ConversationIR (zero behavior change)`。

### Phase 2 — 拆为 Phase 2a（已完成）+ Phase 2b（待执行，零行为 integration/plumbing）[G6]
> **范围裁决（Leader 采纳报告 45 决议）**：已提交实现是 **additive foundation**，而方案原文把 integration/wiring 也列在 Phase 2，存在范围偏差。正式裁决：**Phase 2a = 已完成（commits `ae5a884`、`39c89b7`，additive foundation）；Phase 2b = 新增、待执行（4 个零行为 integration commits）；Phase 3 仍负责行为激活**。报告 43（S1/S2/S3）与 44（MUST_FIX #1/#2、SHOULD_FIX #1-#4）发现的缺口已**正式重分类**为 Phase 2b 交付物，**不是已完成项**；**不回滚已提交的 Phase 2a**。**G2 / 真实 A-B 仍不是 Phase 2b 前置**（离线/只读并行证据，见 Phase 5 修订；只门控激活与发布）。

#### Phase 2a — additive foundation（✅ 已完成，勿重做；commits `ae5a884`、`39c89b7`）
- `ae5a884` `feat: add provider-neutral cache telemetry`（6 文件，+713/−25）：新增 `src/cache.rs`（`CacheSource`/`CacheStats`/`from_chat_usage`/`from_responses_usage`/optional adapters/`derive_miss`）、`src/schema.rs`（自 request.rs 提共享 `canonical_hash/canonicalize`，字节等价）；`src/openai/types.rs` 增 `Usage.cached_tokens: Option<u32>`（**仅 Deserialize**、无 Serialize ⇒ 出站 wire 零变化）；`src/reasoning/prefix.rs` 增 `compute_prefix_fingerprint_v2`（版本化完整前缀哈希，v1 及其调用者字节不变）；`src/responses/request.rs` 复用 `schema::canonical_hash`（纯内部 −26/+1）；`src/main.rs` +2 mod。
- `39c89b7` `test: cover cache usage adapters`（+1，converter golden 测试字面量 `cached_tokens: None`）。
- **性质**：全部新项为 `#[cfg_attr(not(test), expect(dead_code))]`，生产**零调用者**，纯观测/纯类型；`cargo test --locked --all-targets` = **138 passed / 0 failed**（报告 43/44 独立复核），fmt/diff-check/clippy `-D warnings` 全 0。**NO-GO（G6）以最强形式满足**（telemetry 完全未接线 ⇒ wire/log 必然不变）。
- 报告 43/44 结论：**无 MUST_FIX 级代码缺陷，不回滚**；唯一实质发现（S1 / MUST_FIX #1/#2）是**范围偏差**，由本裁决收口为 2a/2b 拆分。

#### Phase 2b — integration/plumbing（待执行，4 个零行为 commits）[G6]
> **总则**：全部为**零行为 integration**——非 opt-in provider 的 wire/log 字节在**每一步**都不变（golden T01-T03 per-wire 持续绿 = 证明）；`cache_policy` 缺省 `None` ⇒ 每处取 legacy 分支；每 commit 独立 `git revert` 干净。**Phase 2b 预期新增测试数标为「待实现」**——实现时按 commit 分解并如实记录，**本文档不预先断言具体数字**（不伪造已完成测试）。测试沿用 in-module `#[cfg(test)]`（binary-only crate：无新 deps、无 `src/lib.rs`）。

- **2b.1** `src/config.rs` `ProviderConfig` 增 `cache_policy: Option<CachePolicy>`（`#[serde(default)]`，缺省=全 off，零行为）+ `src/cache.rs` 增 `CachePolicy`/`UsagePolicy` 类型（`#[serde(default)]` 全字段；`cache_usage_enabled()` 选择器）+ **validate hook**（仅 `upstream=Some` 时名校验；**不做 effort fail-fast**——effort fail-fast 是 Phase 3 启动失败行为，G3 前禁做）+ 全部 `ProviderConfig` 结构体字面量补 `cache_policy: None`（config.rs 内置、test_support.rs、converter/request 测试字面量；compiler 强制枚举）。**旧 config 兼容**：`ProviderConfig` 仅 Deserialize、serde 忽略未知字段 + `#[serde(default)]` ⇒ 现有 config.toml、live `/etc/cc-proxy/config.toml`（不动）、全部 TOML fixture 解析不变，**2b 不需要改任何 config 文件**。测试：config 解析 default-off、upstream-key validate hook。**NO-GO**：任何 wire/log 变化（G6）。
- **2b.2** `src/openai/types.rs` `ChatCompletionRequest` 增 `prompt_cache_key: Option<String>`（`#[serde(skip_serializing_if="Option::is_none")]`，None ⇒ 出站零变化）+ `src/cache.rs` 增**纯函数** `session_key(req, policy, model, provider) -> Option<String>`（§3.3 精确实现：`sha256(upstream|model|source_name|source_value)[..16]`；源优先级 `metadata.user_id` → (未来) 入站 header → **绝无 UUID 兜底** → 缺源 `None` fail-closed）+ **T16-T19 fail-closed 契约测试**（T16 None⇒wire 无字段 / T17 同 session 相等 / T18 重连相等 / T19 异 session 不同 + 哈希不进明文日志）+ serde round-trip。**只建字段/helper，不注入**（注入 = Phase 3 行为）；**Responses encoder 不加该字段**（Kimi 是 Chat wire，Responses 仅 `gpt-5.6-luna`；记为显式非目标防蔓延）。**NO-GO**：None 时出站出现字段；key 明文进日志。
- **2b.3** Responses 侧统一：`src/cache.rs` 增共享 selector `responses_usage_view(usage, policy)`；`src/responses/response.rs` + `src/responses/stream.rs` 经 policy 接线（`cache_stats_from_usage`/`terminal_usage` 委托 view；`cache_stats_from_values` → 更名 `legacy_cache_stats_from_values` 作 legacy 分支）。**CacheStats 命名裁决**：`cache::CacheStats`（canonical raw，opt-in）与 `responses::response::CacheStats` **改名 `LegacyCacheStats`**（纯内部 rename，log 字段串/wire 输出不变）解决名冲突；**Legacy vs Raw 互斥**——单一选择器（`enum CacheStatsMode { Legacy, Raw }`，由 `policy.cache_usage_enabled()` 决定）每请求**只算一个 view，绝不双发**。**测试**：Responses 非流 + 流矩阵（`response.completed` usage 驱动 `terminal_usage`）read/write/miss 分桶 + legacy 分支数值保留（T25/T27 流维 = **2b MUST**，报告 44 SHOULD_FIX #3）。
- **2b.4** Chat 侧统一：`src/cache.rs` 增 `chat_usage_view(usage, policy)`；`src/openai/converter.rs` + `src/sse/stream.rs` 经 policy 接线（`convert_non_stream_response` L64-87、`SseStateMachine::finalize` L389-412、KV log L340-361 都由 view 输出）；`src/routes/messages.rs` 从 `Config.provider_config(profile.provider).cache_policy` 一次查询**下传**（`handle_messages` → `convert_non_stream_response` + `SseStateMachine::new`）——**全程无 `if provider=="moonshot"`/`starts_with` 字符串**（G7 保持）。**清理时机**：`from_chat_usage`/`from_responses_usage`/`from_optional_*`/`CacheSource`/`CacheStats` 的 `#[cfg_attr(not(test), expect(dead_code))]` 与「nothing wired」头注释在**加入生产调用者的同一 commit（2b.3/2b.4）内移除**，绝不事后单独清理（否则 clippy `-D warnings` 因 `unfulfilled_lint_expectations` 失败）。**测试**：Chat 流（usage-only final chunk）+ 非流，opt-in vs legacy 双跑（`test_config()` vs 新 `test_config_opt_in()`）；**非 opt-in legacy wire/log 不变**（golden 持续绿 = 证明）；**Chat `prompt−cached` remainder 对 legacy 永不重标**（legacy 仍 `Some(p−c)`/clamp-0 原样保留并文档化），**修正语义（creation=None、remainder=miss、cached>prompt ⇒ miss None）仅 policy-gated opt-in 生效**。**NO-GO**：任一非 opt-in Chat wire/log 字节变化（G5/G6）。
- **提交**：4 个独立 commit（2b.1/2b.2/2b.3/2b.4），每步 `cargo fmt --check`、`git diff --check`、`cargo test --locked --all-targets`、`cargo clippy --locked --all-targets --all-features -- -D warnings` 全 0。**NO-GO（G6）**：任何未 opt-in provider 的 tools/wire/usage/log 输出字节变化；任一 golden 字节不一致（G5）。

### Phase 3 — provider 名归一 + effort 校验 + optional key（commit 3，config-only + 少量 plumbing）[G3][G4]
> **依赖说明（范围裁决后修订，报告 45 Q1/Q7）**：Phase 3 **仅做行为激活**——provider canonicalization/upstream 绑定（C11/G9）、effort enum fail-fast（C3/C12）、`prompt_cache_key` key 注入（C1，fail-closed）、repo `config.toml` `cache_policy` 声明。**Phase 2b 已把所需基础字段/类型/纯 helper 全部建成**（`cache_policy` 字段 + `CachePolicy` 类型、`prompt_cache_key` 字段 + `session_key()`、`chat_usage_view`/`responses_usage_view` 选择器、`LegacyCacheStats` 更名）；Phase 3 **不再承担缺失的基础字段/类型**（报告 43 S1 / 44 MUST_FIX #1 的缺口已重分类进 2b）。**前置门**：Phase 2b 全门绿（G5/G6）后才允许进入 Phase 3；本 Phase 仍受 G3/G4/G7 门控。
- **provider 名归一（C11，G9）**：policy `upstream` 绑定取代 `select_client` 的字符串匹配（routes/messages.rs L60-70）；三套名（fireworks/moonshot/moonshot-official）收敛为**单一 canonical 名**（推荐 `moonshot`，全链统一：config.rs 内置默认、config.toml profile provider、routes 分发、policy 表）。
- **effort 校验（C3/C12）**：`validate()` 增 effort_enum 校验——映射结果 ∈ 官方枚举 `{low,high,max}`，否则启动 panic / 拒绝 / 归一（策略选定其一，**silent-fail 原则**）；`apply_effort_direct`（converter.rs L191-261）保证 Kimi 侧无非法值（抓 `medium→非法`）。
- **optional key（C1，fail-closed，注入 = 行为）**：`cache.rs::session_key`（**2b.2 已建**）+ **Chat encoder 尾段注入**（anthropic/converter.rs L34-187 尾段，`if let Some(p)=&policy { if let Some(k)=cache::session_key(...) { openai_req.prompt_cache_key = Some(k) } }`）；`handle_messages` 无改动（metadata 来源），header 来源留待后续；**Responses 不注入**（Kimi 为 Chat wire，2b.2 已记为显式非目标）。
- `config.toml` `[providers.moonshot]` 增 `cache_policy`（`effort_pin=false`、`replay=off`、`history=off`、`usage="top_level_cached_tokens"`、`upstream="official"`）——**仅声明，不激活行为 gate**。
- 文件：`src/config.rs`、`src/cache.rs`（session_key 注入调用）、`src/routes/messages.rs`、`src/anthropic/converter.rs`、`config.toml`（**不再含 `src/responses/request.rs`**——Responses 不注入，见 2b.2 非目标）。
- 测试：**T16-T19 fail-closed 契约测试已在 2b.2 建成**（缺源→字段省略 / 同 user 相等 / 重连不变 / 异 user 不同且不进明文日志）；Phase 3 补**注入路径行为断言**（opt-in 时字段出现、值 = §3.3 派生、非 opt-in 仍省略）；effort 校验（T11-T13）；provider 名归一后 `select_client` 无字符串特判。
- **NO-GO（G3）**：未读回 live `/etc/cc-proxy/config.toml` 确认 effort_map 与 moonshot provider 设置 → 不激活。
- **NO-GO（G4）**：session 稳定 key 契约未按 §3.3 落成可执行契约 → 不注入。
- **NO-GO（G7）**：不得新增任何 `if provider=="moonshot"`/`starts_with`；`select_client` 的 `"moonshot-official"` 字符串匹配必须被 policy `upstream` 取代（或显式标注为临时容忍）。
- 提交：`feat(cache): canonicalize provider key + effort enum validation + optional session key (fail-closed)`。

### Phase 4 — 行为 gate（moonshot 仅，拆 4 子提交，每子步单变量归因 + 独立回滚）
> 依据：04 探针「最小充分条件」含完整回放 + append-only，但**必须单变量归因**，不能一次全上。

| 子步 | 内容 | 作用域 | 测试 | NO-GO |
|---|---|---|---|---|
| **4a** | `effort_pin_per_session=true`：首个 effort 钉住，同 session 不因 thinking budget 抖动而换 effort | moonshot 仅 | T13 | 非 moonshot 行为不变；行为变更须 G3 后开 |
| **4b** | `replay="full_assistant"`：assistant 轮回放独立于当前请求 thinking（消灭 converter.rs L43-60 重算）；`placeholder`：kimi=`keep`（未受控验证前不 `omit`），deepseek=**保持**占位符（DeepSeek 对 tool_calls 无 reasoning_content 会 400） | moonshot 仅 | T06/T07 | DeepSeek/GLM 占位符行为不变 |
| **4c** | `history="append_only"`：gate 掉 cleanup（build_messages.rs L135/L357-447）、compact（L14-39）、去重（L90-96） | moonshot 仅 | T08 | GLM/DeepSeek 依赖这些安全网，维持现状 |
| **4d** | Chat relocate 切换 `migrate` → `split_volatile_system_blocks`+合成尾部（relocate.rs L212-231 / request.rs L139-154）；`CODEMERMAFROST_RELOCATE` env 门控被 policy `relocate` 取代或显式 OR（保留两套开关是事故源） | moonshot 仅 | Chat kimi 端到端 fixture（含 alternation 验证：Chat 合成尾部落 tool 角色后不违反 OpenAI alternation） | 单独提交；kimi wire 变化允许，其他 provider 禁止 |

- 每子步：golden/parity 只对 kimi 放开；hit-rate telemetry 前后对比（同 session 同池 R2+ `cache_read > 0` 且维持高位；**随轮递增仅作观察指标，不作 MUST**）；其他 provider 字节不变。
- **风险点（必须写进测试）**：
  - full_assistant replay + 无 reasoning 的 tool-call 轮：Kimi 是否像 DeepSeek 一样 400 需受控探针确认 → 未确认前 `placeholder="keep"` 兜底。
  - append_only 关闭 cleanup/compact 的代价：放弃对「孤儿 tool_calls」「超大 tool_result」「重复结果」的防御 → 限制在 kimi 且保留 telemetry 异常计数。
  - Chat 合成尾部 alternation：Responses 合成尾部是独立 `user` item（request.rs L150-153）；Chat 现 migrate 是追加进最后一个 user 内容（wire 形态不同）→ 4d 必须带 kimi 端到端 fixture。
- 文件：`src/config.rs`（policy 值）、`src/reasoning/build_messages.rs`、`src/reasoning/should_replay.rs`、`src/reasoning/relocate.rs`、`src/anthropic/converter.rs`、`src/reasoning/sanitize.rs`、`config.toml`。
- 提交：4 个独立 commit（`feat(kimi): pin effort per session` / `feat(kimi): full assistant replay` / `feat(kimi): append-only history` / `feat(kimi): split-tail relocate`），每个可单独 revert。

### Phase 5 — 回归与 parity + live A/B（commit 5）[G2 离线只读证据，可随时并行]
> **G2 定位（修订）**：G2 是**可随时进行的离线/只读 outbound 证据**（cc-proxy→clawbot:11434 抓包 + 前缀 hash + cached 配对 + 后端切换时间线），**任何时点（含 Phase 1-4 期间）均可并行执行**；**它不是 Phase 1/2/3/4 的代码前置**（Phase 1-4 均不依赖 G2）。G2 产出用于**生产 0% 根因升格**（A1 CONDITIONAL→HIGH）与**最终发布门 B8**；G2 未完成只影响发布/根因判定，不阻塞 Phase 1-4 代码推进。
- 全量 `cargo test --locked --all-targets`；Chat/Responses × 非流/流 × relocate on/off **per-wire** 字节 parity 矩阵；确认 DeepSeek/GLM/GPT **零变化**（回归 golden 不变，22 G-parity）。
- 真实 A/B（§8）：direct upstream（直连 clawbot:11434）vs staging proxy（11449 隔离）同 session 同前缀对照；cache 四桶独立计数；HTTP 4xx/5xx/timeout 与 miss/creation 不混淆（22 T35/T36）。
- **NO-GO**：任一既有 provider 的 wire/SSE/usage 输出变化；A/B 未过 → 不发布。
- 提交：`test: full parity matrix + live A/B report`（A/B 报告不入代码）。

---

## 7. 测试矩阵（MUST / SHOULD / OPTIONAL）

> **修正声明**：旧 PLAN §5「golden：IR 迁移后 Chat↔Responses 同 fixture 字节一致」**不可成立**（Chat 与 Responses 词汇不同，工具/推理场景字节必然不等 → 该门永远不绿 = 没门）。已改为 T01-T03 per-wire 门 + T04 cross-wire 语义 parity。

### 7.1 Golden / wire 字节门
| ID | 层级 | 测试 | 断言 |
|---|---|---|---|
| T01 | **MUST** | Chat 旧编码器 vs 新 IR 编码器，fixture 全集（text/thinking/tool/多轮/refusal/image/null） | 字节相等（per-wire golden-1） |
| T02 | **MUST** | Responses 旧编码器 vs 新 IR 编码器，fixture 全集 | 字节相等 |
| T03 | **MUST** | 同输入重复编码（同线程/跨线程） | 字节相等（确定性 golden-3；隔离 env var 竞争） |
| T04 | **SHOULD** | Chat↔Responses 同 IR 语义 parity（内容序列/角色/工具往返） | 语义等价，非字节 |
| T05 | **MUST** | `CODEMERMAFROST_RELOCATE` on/off 两态各捕 golden | 各自稳定、互不串扰 |

### 7.2 Chat 多轮内容
| ID | 层级 | 测试 | 断言 |
|---|---|---|---|
| T06 | **MUST** | 多轮 text-only：S+U1+A1+U2+… | 前缀字节逐轮稳定（append-only） |
| T07 | **MUST** | thinking+text 多轮：assistant 完整回放 reasoning+text | replay 独立于当前 effort；占位符路径在 full_assistant 策略下禁用（kimi=keep） |
| T08 | **MUST** | tool_call→tool_result 多轮（含多工具并行、is_error=true） | wire 稳定；orphan cleanup/compact 在 append_only 下 gate 掉 |
| T09 | **SHOULD** | redacted thinking / refusal / image / null content / 空 messages | 与旧行为字节一致（或明确策略） |
| T10 | **SHOULD** | SystemPrompt::Text vs Blocks、多 system 块 | 稳定序列化 |

### 7.3 Kimi effort
| ID | 层级 | 测试 | 断言 |
|---|---|---|---|
| T11 | **MUST** | effort_map 全键输出 ∈ {low,high,max} | 无非法值（抓 medium→非法） |
| T12 | **MUST** | effort_enum 校验：配置含非法值 → 启动失败/归一 | 配置校验不静默 |
| T13 | **MUST** | effort pin per session：thinking budget 抖动不换 effort | 同 session 字节稳定 |
| T14 | **SHOULD** | 不同 effort → static_prefix_hash 不同 | 指纹区分 |
| T15 | **OPTIONAL** | effort=off 映射 Kimi 最低档（disable_thinking=true） | =low |

### 7.4 prompt_cache_key
| ID | 层级 | 测试 | 断言 |
|---|---|---|---|
| T16 | **MUST** | 无 session 上下文 | wire 无 `prompt_cache_key` 字段（fail-closed） |
| T17 | **MUST** | 同 session 多轮 | key 逐轮字节相等 |
| T18 | **MUST** | 重连（新连接/进程重启） | key 不变 |
| T19 | **MUST** | 异 session | key 不同；不进明文日志（哈希） |
| T20 | **SHOULD** | key 变化 vs 前缀命中（对照 04 C1） | key 非命中充分条件，命中率语义不混淆 |

### 7.5 tools canonicalization
| ID | 层级 | 测试 | 断言 |
|---|---|---|---|
| T21 | **MUST** | 同 schema 不同键序（含嵌套 properties/description/items） | 字节相等 |
| T22 | **MUST** | 工具数组序稳定（Chat/Responses 各自排序） | 跨轮稳定 |
| T23 | **SHOULD** | required/enum 数组序策略 | 按定义序相等/不等各一例 |
| T24 | **SHOULD** | 指纹含完整 schema 内容（升级 prefix.rs） | 跨轮指纹相等、内容差异指纹不同 |

### 7.6 usage 字段
| ID | 层级 | 测试 | 断言 |
|---|---|---|---|
| T25 | **MUST** | Kimi 顶层 `usage.cached_tokens` 读取 | 映射入 cache_read_input_tokens |
| T26 | **MUST** | DeepSeek `prompt_cache_hit/miss_tokens` 接入 | 不双算、映射正确 |
| T27 | **MUST** | Responses `input_tokens_details` read/creation/miss 三桶 | 不重叠、算术正确 |
| T28 | **MUST** | 边界：cached>prompt、prompt=0、creation 与 miss 分离 | 无 panic/负值、分类不混淆 |
| T29 | **SHOULD** | 四 provider usage 形状汇总 | 统一 telemetry 输出一致性 |

### 7.7 GPT Responses items
| ID | 层级 | 测试 | 断言 |
|---|---|---|---|
| T30 | **MUST** | instructions 有无/空/多块 | 正确进 `instructions`；无空串 |
| T31 | **SHOULD** | function_call_output 多轮续写、output 对象/字符串 | 正确往返 |
| T32 | **SHOULD** | refusal / 空 output / status cancelled | 正确映射或错误 |

### 7.8 stream / error / EOF / call ID
| ID | 层级 | 测试 | 断言 |
|---|---|---|---|
| T33 | **MUST** | Chat 流：EOF 无 terminal、`[DONE]` 无 finish_reason、idle timeout、缓冲超限 | 正确 finalize/error，不悬空 |
| T34 | **MUST** | 真实 call ID 全程一致（msg_{20hex}）；`message_start.usage` 对象契约（两 wire） | 不重生成；不 null |

### 7.9 生产验收（非单元）
| ID | 层级 | 测试 | 断言 |
|---|---|---|---|
| T35 | **MUST** | direct-upstream vs proxy A/B 同 session（staging 11449） | 同后端**稳定会话** R2+ `cache_read > 0` 且维持高位（对齐 04 探针 92.7–100% 量级）；四桶独立计数。**严格单调仅作观察指标，不作 MUST**（真实上游 eviction/TTL 可破坏单调） |
| T36 | **MUST** | HTTP 4xx/5xx/timeout 分类 | 与 miss/creation 不混淆 |
| T37 | **SHOULD** | manifest 校验（**provenance-aware**）：二进制 SHA == 源码 commit 重建 | 仅**同 commit + 同 toolchain + 同 flags + 同环境**内要求 byte SHA 相等；跨环境/历史构建以 ELF 可加载/功能段 hash、反汇编/行为验证和 manifest 为准，**不把跨环境 byte mismatch 当代码差异** |
| T38 | **SHOULD** | 回滚演练：revert Phase 后 A/B 回到基线 | 行为/命中率回归基线 |

---

## 8. 真实验证与发布/回滚

### 8.1 验证链路与端口隔离
- **direct upstream**：`http://clawbot:11434/v1/chat/completions`（直连，`NO_PROXY=*`，仅用于受控探针/A-B 对照；沿用 04 probe.py 方法：固定 model/effort/byte 稳定前缀/完整回放）。
- **staging proxy（11449 隔离）**：`LISTEN_ADDR=0.0.0.0:11449` + **独立 config**（上游指向 clawbot:11434、含 Phase 3 cache_policy）启动第二个 cc-proxy 实例做 A/B；**生产 11441 全程不触碰**（不重启、不替换 `/usr/local/bin/cc-proxy`、不改 systemd、不改 `/etc/cc-proxy/config.toml`）。11449 现无监听（17 报告），验证结束后确保无残留进程。
- **四桶独立计数**：creation / read-hit / miss / HTTP-error 互不重叠（对齐 04 §5）；不把 miss 记成 error、不把 creation 记成 miss。

### 8.2 门控（实现/激活/发布各层）
| 门 | 判定 | 阻塞 |
|---|---|---|
| G1（已解除） | dirty 工作树已固化至 origin/master `31f9b851` | — |
| G2 | 生产 outbound 抓包对照（cc-proxy→clawbot:11434 前缀 hash + cached 按 request id 配对 + 后端切换时间线） | **生产 0% 根因升格**；未做前不得改路由 |
| G3 | 读回 live `/etc/cc-proxy/config.toml` 确认 effort_map 与 moonshot provider 设置 | **Phase 3 激活** |
| G4 | session 稳定 key 契约（§3.3）已定义 | **key 注入** |
| G5 | Phase 1 golden（T01-T03 per-wire）字节一致 | **整体 NO-GO** |
| G6 | schema.rs 不得改变未 opt-in provider 的 wire 字节 | Phase 2 |
| G7 | 无新增 kimi 硬编码；select_client 字符串匹配被 policy 取代 | Phase 3 |

### 8.3 发布与回滚
- **每 Phase 独立 commit**；NO-GO → revert 该 Phase、不发布。
- **发布 = 合并 feature 分支到 `origin/master` → 从 `git archive origin/master` + `cargo build --release --locked` 重建 → manifest 比对 → 部署 staging → A/B 验收 → 授权后部署生产**。
- **线上回滚 = 重部署 `origin/master` 二进制**（勿用本地 `master@f6425e8`——本地滞后，以 `origin/master` 为真）。
- **Artifact provenance（19 §5，随 artifact 同目录落 MANIFEST.txt/json）**：
  - source commit：`git rev-parse origin/master` 全量 SHA + tree SHA（`^{tree}`）+ 导出方式（`git archive`）；
  - toolchain：rustc/cargo 版本（1.97.1）、LLD、gcc（`readelf --string-dump=.comment`）、Cargo.lock 是否 `--locked`；
  - build：`cargo build --release --locked`、CARGO_TARGET_DIR 隔离、宿主 + 时间戳；
  - binary：sha256 + size + BuildID（`readelf -n`）+ `file`；
  - config：config 版本 + `/etc/cc-proxy/config.toml` sha256 + 部署路径；
  - 回滚点：`cc-proxy.before-<ts>` sha256 入 `/data/backups/cc-proxy/<ts>/`。
- **Hash 对比纪律（19 §5.3）**：源码级一致看 tree hash；功能一致看 ELF 逐节 hash；字节 hash 仅同 provenance（**同 commit + 同 toolchain + 同 flags + 同环境**）才有效，跨环境/历史构建以 ELF 可加载/功能段 hash + 反汇编/行为验证 + manifest 为准，**不把跨环境 byte mismatch 当代码差异**。
- **保留回滚证据**：live 二进制 + 全部 `.bak-*`/`/data`/`/var` 备份 + `/tmp` 重建物证，在「新构建验收通过 + SHA256SUMS 归档」前**不得删除**（18 §4）。

### 8.4 发布最小阻断（B1-B10，全部 MUST）
| # | 条件 | 不满足则 |
|---|---|---|
| B1 | 全量 `cargo test --locked --all-targets` 绿（基线 101 + 新增矩阵） | 不发布 |
| B2 | T01-T03 golden 字节门全绿（每 Phase） | 该 Phase 不合并 |
| B3 | effort_enum 校验生效且 Kimi 全 effort 值合法（T11-T13） | 不激活 Phase 3 |
| B4 | prompt_cache_key 四态契约测试绿（T16-T19）；G4 已定 | 不注入 key |
| B5 | provider canonical 名统一（G9/C11） | 不发布 |
| B6 | 非 moonshot provider 回归 golden 不变（Phase 5 parity） | 不发布 |
| B7 | usage 四 provider 映射测试绿（T25-T28），creation/miss 不混淆 | 不发布 |
| B8 | G2 生产抓包对照 + A/B（T35/T36）通过；HTTP 错误分类独立 | 不发布（0% 根因仍 CONDITIONAL） |
| B9 | manifest（**provenance-aware**）：同 commit+toolchain+flags+环境 内二进制 byte SHA == origin/master 重建 SHA；跨环境/历史构建以 ELF 功能段 hash + 反汇编/行为验证 + manifest 为准（**不把跨环境 byte mismatch 当代码差异**）；回滚演练通过 | 不发布 |
| B10 | **部署后 11441 行为级回归（U8 关闭；Leader 显式授权后执行）**：部署 master 重建二进制后，至少验证——moonshot-official 路由可达（官方客户端请求走通）、`message_start.usage` 非 null（两 wire）、Chat 与 Responses 关键路径（非流/流、usage 字段）行为等价。**当前尚未执行**（无部署授权，见报告 20 §6.6/U8） | 不发布 |

---

## 9. 压缩上下文后的开发入口（新会话直接可执行）

### 9.1 工作区与路径
- 仓库根：`/root/projects/codewhale-proxy/source`（remote `origin` = `https://github.com/zwczwczwc/cc-proxy.git`）。
- 基线 commit：`origin/master` = `31f9b851308d2845b69d35880e35e1805b8e4f18`（tree `6e1a2132`）。
- feature 分支：`feat/kimi-k3-cache-optimization`（@ 31f9b851，未 commit/push）。
- **本文件 = 唯一开发主入口**；配套 `KIMI-K3-CACHE-OPTIMIZATION-CONTEXT-RECOVERY.md` = 状态恢复；旧 `KIMI-K3-CACHE-OPTIMIZATION-PLAN.md` 仅作历史参考（以其与本文冲突处按本文为准）。

### 9.2 首轮读取顺序（新会话第一步）
1. **本文档**（§0-§9）——决策与契约；
2. `KIMI-K3-CACHE-OPTIMIZATION-CONTEXT-RECOVERY.md`——当前状态/首轮只读命令；
3. 报告（按需，位于 `/tmp/shared/kimi-cache-hit-issue/deepseek-reports/`）：
   - `20-research-facts-matrix-*`（事实/假设/未知）、`21-architecture-review-*`（架构裁决）、`22-acceptance-matrix-review-*`（测试矩阵裁决）、`18-difference-adjudication-*`（provenance 表述）、`19-single-source-governance-*`（发布治理）、`04-*`（探针原始结论）、`08-*`（Phase 0 落地）。
4. 源码核对（按 Phase 需要，行号见 §6）：`src/config.rs`、`src/anthropic/converter.rs`、`src/responses/request.rs`、`src/openai/types.rs`、`src/reasoning/build_messages.rs`、`src/reasoning/prefix.rs`、`src/reasoning/relocate.rs`、`src/routes/messages.rs`、`src/sse/stream.rs`、`src/responses/{response,stream}.rs`、`config.toml`。

### 9.3 首轮只读命令
```bash
cd /root/projects/codewhale-proxy/source
git status --short --branch
git rev-parse HEAD origin/master
git ls-remote origin refs/heads/master          # 与 origin/master ref 比对
git branch -vv                                  # 确认 feat/kimi-k3-cache-optimization @ 31f9b851
git stash list                                  # stash@{0} 勿 pop/覆盖
git status --porcelain | grep -c '^??'          # 预期 32（31 旧 + 本 FINAL-PLAN）
git diff --stat                                 # 预期空（tracked=0）
cargo test --locked --all-targets               # 基线 101 passed（首次可跑，只写 gitignore 的 target/）
```
若 `origin/master` 与远程不一致 → 仅允许 `git fetch --no-tags origin master`（refs-only）；**严禁 pull/rebase/reset/clean/checkout 覆盖未跟踪文件**。

### 9.4 分阶段委派方式（每 Phase 一个子 Agent）
- 每 Phase 独立委派一个子任务，子任务书必须包含：**Phase 编号与目标、本文件对应 §/表格、精确文件路径、依赖门（G 编号）、NO-GO 判定、提交信息模板、回滚方式**。
- **Phase 1 委派书必须钉死 golden harness 结构（27 SF-1）**：in-module `#[cfg(test)]` + `tests/golden/*.json` 仅作数据（`CARGO_MANIFEST_DIR` 读取），**不新增 `src/lib.rs`**（binary-only crate 下 `tests/` 集成测试无法链接内部符号，否则首个子任务编译即卡）。
- 子 Agent 产出：commit（每 Phase 一个或 4 子步 4 个）+ 一份报告落 `/tmp/shared/kimi-cache-hit-issue/deepseek-reports/<NN>-phase<N>-*-<model>.md`。
- Phase 间串行：前一 Phase NO-GO 未绿 → 后一 Phase 不得开始。

### 9.5 TDD / 独立 review 规则
- **TDD**：每 Phase 先写测试（golden/parity/契约）再改实现。**Phase 1 例外**：这是**零行为重构**——先捕获旧 encoder golden 作为**绿基线**，IR 迁移后仍绿即通过，**不强制制造 RED**（重构不该变行为；变红 = 行为漂移 = NO-GO）。RED→GREEN→REFACTOR 仅适用于 **Phase 2-4 的新行为开发**。golden 必须在重构**前**捕获（Phase 1 顺序关键）。
- **独立 review**：每 Phase 合并前由独立 reviewer 子 Agent 按 `review-verification-protocol` / `code-review` 技能复核：security 扫描（无密钥/无 kimi 硬编码/无随机 UUID key）、golden 证据核对、policy 只经 config、`cargo clippy --locked --all-targets --all-features -- -D warnings` 0 警告、`cargo fmt --check` 仅基线 nit。
- **环境竞争**：含 `CODEMERMAFROST_RELOCATE` 等进程级 env 的测试需串行/显式隔离（golden-3）。

### 9.6 停止条件（任一条满足即停并上报 Leader）
1. 任一 NO-GO 门被触发（G5/G6/G7/G3/G4 未满足强行推进）；
2. golden/parity 任一 fixture 字节不一致；
3. 未 opt-in provider 的 wire/SSE/usage 输出变化；
4. `cargo test --locked --all-targets` 非全绿；
5. 需要触碰生产（11441 / `/usr/local/bin/cc-proxy` / `/etc/cc-proxy` / systemd）而未获显式授权；
6. 需要读取 live config（G3）而无 root 受控只读授权；
7. 发现本文档与源码事实不符 → 更新本文档并记录，不得静默继续。

---

## 10. 安全边界（全程遵守）
- 不触碰：`/etc/cc-proxy`、systemd、`/usr/local/bin/cc-proxy`、11441、生产 config、`clawbot` 节点实例（11435）。
- 不执行：commit/push/merge（除非 Phase 授权）、reset/clean、pop 覆盖 `stash@{0}`。
- 未跟踪 .md/tools 不得进入任何提交。
- 文档不含密钥/完整 prompt/完整 schema/reasoning 原文。
- 业务 API 调用仅限：受控探针端点 `clawbot:11434`（synthetic 内容）与 staging proxy 11449（Phase 5 A/B），**从不调用 11441**。

---

## 附：OSS 参考与证据边界（报告 34 OSS-grounded 最终裁决摘要）

> 本方案「站在别人肩膀上」的证据边界已在报告 34 以 `gh api` 逐项实测（固定 commit/tree 存在、无漂移）。**核心边界：没有任何开源项目证明「K3 prefix cache 已解决」**；命中率达成只能靠本项目 golden + 受控探针 + 真实 A/B（T35/B8）。

| 项目（固定 commit） | 对方案的证据等级 | 内容（实测锚点） | 边界（不得升级/复制） |
|---|---|---|---|
| raine/claude-code-proxy `e60cf008…` | **直接** | `prompt_cache_key` 字段/注入（request.rs L33/L159/L654）；full_assistant **无条件完整回放**（push_assistant_message L561-622）；billing 头过滤（L44-48）；K3 effort 分支 L184-200 | ccp 直传明文 session_id（本方案改 sha256 派生，禁抄）；伪造 thinking signature（禁抄）；read_effort 合法集含 medium/xhigh 与官方 {low,high,max} 冲突（D5，禁作依据） |
| musistudio/claude-code-router `fcf3d85d…` | **直接（usage 桶）+ 间接（声明式路由）** | usage 遥测四桶/归一（cacheRead/cacheWrite/cacheRatio + SQLite 落库）来自 PR #1681/#1655/#1461/#1650/#1588（全部实测存在）；preset / routing conditions 声明式 | CCR 对 Kimi 原生透传**零转换**（仅作「零转换对照组」，不得用作转换正确性证明，D1）；CCR `cacheRatio` 定义是 bug（#1588，1000%+，禁抄） |
| m0n0x41d/anthropic-proxy-rs `59eb97bc…` | **间接** | 双 wire 模型 + translate 管线（src/models/{anthropic,openai}.rs + src/translate/）→ 支持 IR + 双 encoder 独立方向 | 与 Kimi/cache 无关（tool_choice 恒 auto、system 清洗为 WAF 403 用途），不得用于证明 cache 方案（D2/D3） |
| CodeWhale / Reasonix / permafrost / dsv4-cc-proxy | **历史来源（非 K3 缓存方案）** | CodeWhale `prefix_cache.rs`（工具排序/SHA-256 前缀指纹，prefix.rs 注释引用）；permafrost（relocate.rs L2-3「Ported from permafrost permafrost_align.py」）；Reasonix / dsv4-cc-proxy 属 cc-proxy 历史功能来源 | **不得与 K3 缓存方案混用**；memory 警示 faceapi.ai 幻觉上游曾被错配——Phase 3 provider 归一（G9/C11）不得引入幻觉名 |

- **本项目证据（显式标注，非「外部最佳实践」）**：golden 体系（T01-T05，外部无输出字节对拍）、受控探针（04，C2a/C2b 对照）、真实 direct/proxy A/B（T35/B8，无外部先例）、四桶分桶实现、`session_key := sha256(provider \| model \| source)[..16]` 派生、`metadata.user_id` 源（CONDITIONAL，须 G2/G3 验证）。
- **GAP-A / GAP-B（本项目证据）**：GAP-A = Kimi 顶层 `usage.cached_tokens` **从未被探针观测**（probe.py L158/L205 只读 `prompt_tokens_details.cached_tokens`）⇒ **不得假定顶层字段存在**（T25 两形状 fixture + G2 同时核对）；GAP-B = 后端归属由返回 model/provider 标签推断（R2/R4 同标签 0%/95%）⇒ A1 保持 CONDITIONAL，G2 补后端切换时间线，**B8 不得放松**。
- **生产 0% 根因证据链**：探针受控对照（HIGH）→ 生产根因 **CONDITIONAL（A1 = eswitch 缓存池隔离）**，须 G2 升格；未升格前不得改路由/改 cc-proxy 之外系统。

## 附：证据轨迹（只读）
- 读取：20/21/22/18/19/08/10/04/07/06 报告、results.json、probe.py、旧 PLAN + CONTEXT-RECOVERY、当前源码（config.rs/converter.rs/types.rs/build_messages.rs/prefix.rs/relocate.rs/routes/messages.rs 等）。
- 独立复核：`git rev-parse HEAD origin/master`、`git branch -vv`、`git status --porcelain | grep -c '^??'`=31、`grep` provider 名三处、`Usage`/`ChatCompletionRequest` 字段确认、config.toml effort_map 确认。
- 写入：本文件 + CONTEXT-RECOVERY + `/tmp/shared/.../23-final-plan-draft-*.md`。
- **状态：MODEL_IDENTITY_UNVERIFIED。** 本方案为文档编制，无业务 API 调用。
