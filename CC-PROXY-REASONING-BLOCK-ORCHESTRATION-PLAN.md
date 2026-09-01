# cc-proxy Responses reasoning block 修复编排计划

## Goal
在不改变 Claude Code 工具调用处理、Responses input 前缀、既有 cache hit 语义和生产配置的前提下，修复 gpt-5.6-luna Responses reasoning summary/SSE → Anthropic thinking block → Claude Code stream-json → cc-connect EventThinking → 飞书显示链路，并用真实隔离 A/B 与 continuation 证据验收；未具备真实 upstream 证据时不得宣称 fully verified。

## 尺寸判断
- 等级：L
- 依据：跨 Rust codec/状态机、Claude/cc-connect 显示链路、cache/input 不变性、真实隔离端口 A/B、工具 continuation、报告与 reviewer 门禁，超过 6 个依赖步骤并涉及 coder/researcher/backend-eng/reviewer。
- 策略：researcher 与 backend-eng 并行审计；coder 在同一现有 dirty 源目录按最小变更执行；三者完成后 reviewer 评审；不通过则按轮次创建修复任务并重新评审。

## 需求门控
- 需求真实性：明确——当前飞书侧看不到 reasoning block，需要修复并证明不影响工具与 cache。
- 现状：源码已有 Responses/cache 改动和历史报告；当前工作树非 clean，需区分既有改动与本轮修改。
- 具体指标：已明确：cargo test --locked、fmt、clippy -D warnings；隔离 11449 A/B；有效 200、cache_read/cache_creation/miss、401/502/504/timeout 分开；至少 3 次独立单工具及必要并行 continuation；Responses terminal/message_stop 闭合。
- 最窄切入：仅修复 reasoning codec/状态机和必要测试，不触碰生产配置、11441、cc-connect 配置或 Chat 路径。
- 可观测性：测试输出、脱敏 wire/SSE/JSONL、二进制与源码对应关系、A/B 原始字段摘要和 reviewer metadata。
- 未来适配性：沿现有 Responses/Anthropic codec 状态机扩展；禁止把前端 thinking_messages/tool_messages 开关耦合到上游请求构造。
- 结论：通过门控，进入并行调研/实现/真实验证。

## 任务树与依赖
1. t_02917184 researcher：审计 reasoning codec、状态机、call_id/cache 缺口（并行）。
2. t_24c3fe26 backend-eng：审计并执行安全的隔离 A/B、cache、terminal、continuation（并行）。
3. t_22a32b7c coder：按 TDD 实现最小修复并交付报告（并行，但必须先保护现有 dirty 工作树）。
4. t_3d4851ca reviewer：依赖 1/2/3 全部 done 后评审；approved=false 时最多 3 轮 Review→Fix→Review。

## 重要上下文恢复清单
- 工作树：`/root/projects/codewhale-proxy/source/`，当前分支 `feat/gpt-responses-transport`，已有大量未提交文件和 `src/responses/`、`tools/`、多份报告；禁止 reset/clean。
- 生产配置：`/etc/cc-proxy/config.toml`；生产二进制：`/usr/local/bin/cc-proxy`；cc-connect 配置：`/home/claude/.cc-connect/config.toml`。
- 约束：不修改生产配置/11441/cc-connect，不切 Chat，不引入常驻代理；隔离验证才可使用临时进程/端口 127.0.0.1:11449，并清理临时资源。
- 既有历史报告不是当前真实 upstream 证据；有效 A/B 必须单独核对 HTTP 状态、cache 字段、TTFT、terminal、message_stop 和真实新 call_id。

## 验收流程
- 等待 t_02917184、t_24c3fe26、t_22a32b7c 状态完成；逐个读取摘要/metadata/实际报告和工作树。
- 只有 reviewer `approved=true` 且无 P0/P1，才进入 Goal Check。
- Goal Check 必须逐项核对：reasoning summary/SSE/readable-vs-encrypted；tool call_id 与 delta；terminal/message_stop；input/cache 不变性；A/B 三个有效样本与错误分类；单工具/并行 continuation 各三次；cargo checks；报告完整性。
- 若 reviewer 不通过：第 1/2 轮按 findings 创建 coder 修复任务并在其完成后创建新的 reviewer；第 3 轮仍不通过则阻塞父任务人工介入。
- 若 A/B 或 continuation 证据不足：不得用静态测试替代；Goal Check 只能标 CONDITIONAL/BLOCKED，并阻塞父任务请求授权/环境准备。

## 当前状态
- [x] 任务恢复、尺寸判断、需求门控、上下文审计
- [x] 并行任务已创建
- [ ] worker 产出
- [ ] reviewer 门禁
- [ ] Goal Check 与最终 complete/block
