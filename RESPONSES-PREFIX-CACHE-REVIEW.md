# Responses Prefix/Cache 实现质量门评审

日期：2026-08-06
评审对象：`/root/projects/codewhale-proxy/source/`
重点文件：`src/reasoning/relocate.rs`、`src/responses/request.rs`
评审类型：代码评审
评审等级：H（核心请求转换/缓存前缀行为，且工作树 diff >100 行）

## 结论

**verdict: conditional**

**approved: false**

本次复评确认 P1 修复已落到 Responses 实际转换入口：动态 context 现在作为独立 synthetic tail，三轮公共历史 wire item 稳定，三层 hash 语义及 usage/cache/status 观测均有实现和单测覆盖；Rust 自动化质量门真实通过。但旁路 mock/clawbot 验证报告（t_762ec998）当前仍在执行，尚未形成独立可复核的最新报告，因此暂不批准最终通过。

## P0 问题

无。

## P1 问题

### [P1] 旁路 mock/clawbot 质量门尚未完成，不能最终批准

**文件/证据**：上游旁路任务 `t_762ec998` 当前状态为 `running`；预期产出 `/root/projects/codewhale-proxy/source/RESPONSES-PREFIX-CACHE-BYPASS-VALIDATION.md` 尚不存在。

**影响**：源码级单测已证明转换与观测逻辑，但尚未有本轮独立的实际 mock wire、tool continuation、流式 Responses、cache miss 与 502/504/timeout 区分的旁路证据；因此只能 conditional。

**建议**：待 t_762ec998 生成真实脱敏报告后，核对 11449 清理、11441 未重启、三轮公共 wire prefix、tool continuation、streaming、Responses 不 fallback 及错误分类；报告缺失或关键场景失败时保持 conditional。

### [P1] 回归测试没有测试实际 Responses 转换，当前测试是无效的“自证”

**文件/行号**：`src/reasoning/relocate.rs:318-336`

**状态**：已修复。当前 `src/responses/request.rs:288-302` 通过 `convert_request_with_relocation()` 实际构造三轮 wire，并比较公共历史 item；synthetic tail 也被断言为独立末项。

**复核建议**：保持该测试覆盖普通历史与 tool_use/tool_result 历史，避免退回仅测试 `split_volatile_system_blocks()` 的弱断言。

### [P1] 三层 hash 只有实现和日志，没有覆盖语义的自动化测试

**文件/行号**：`src/responses/request.rs:46-101`

**状态**：已修复。`src/responses/request.rs:304-320` 覆盖 canonical key order、tail 变化时 history/wire hash 独立变化、无 tail 时 history/wire 一致；实现同时对 tools 排序并记录三层 hash。

### [P1] 质量门要求的 cache/上游观测指标未在实现中落地

**文件/行号**：`src/responses/response.rs:78-86`、`src/client.rs:108-165`

**状态**：实现已补齐。响应转换记录 input/output/cache read/cache creation token；请求 telemetry 记录 hash、item types、synthetic tail，失败路径保留 HTTP status 且错误体脱敏。最终仍需旁路报告验证这些字段在真实链路可观察且不泄露 Authorization/prompt。

## P2 问题

### [P2] 纯拆分 API 的单测覆盖仍偏少

**文件/行号**：`src/reasoning/relocate.rs:211-230`

当前测试间接覆盖了 volatile block，但没有直接覆盖：稳定 block 与 volatile block 混合返回、`SystemPrompt::Text` 原样返回、无 volatile block 时返回原 system。建议补齐这些边界测试，避免未来修改检测器时回归。

### [P2] `migrate_volatile_system_blocks` 的旧 Chat 路径仍保留易误用 API

**文件/行号**：`src/reasoning/relocate.rs:125-208`

Responses 已不调用该函数，Chat 行为保持现状符合本次范围；但函数注释仍将“追加到 latest user message”描述为通用 relocation 行为，容易被新调用方误用于跨轮 Responses。建议在文档注释中明确其仅为旧 Chat 兼容路径，Responses 必须使用 split + synthetic tail。

## 自动化检查（本次真实运行）

- `cargo test --all-targets --locked`：通过，76 passed，0 failed。
- `cargo fmt --all -- --check`：通过，exit 0。
- `cargo clippy --all-targets --all-features -- -D warnings`：通过，exit 0。
- `git diff --check`：通过，exit 0。

## P1 复核结论

- Responses 实际转换入口已覆盖：`src/responses/request.rs:17-109` 在 relocation 开启时先转换完整历史，再追加独立 synthetic tail，不回写历史 message。
- 三轮公共 wire 历史稳定测试已覆盖：`src/responses/request.rs:272-302`，同时含 tool_use/tool_result 历史。
- 三层 hash 语义测试已覆盖：`src/responses/request.rs:304-320`；canonical object key 排序、history 排除 tail、wire 包含 tail、无 tail 时 history/wire 一致均有断言。
- cache/token/status 观测已实现：请求 telemetry 位于 `src/client.rs:108-165`，响应 usage/cache 记录位于 `src/responses/response.rs:78-86`；Authorization 与错误体经脱敏处理。
- 旁路 mock/clawbot 任务 `t_762ec998` 仍为 `running`，预期报告 `RESPONSES-PREFIX-CACHE-BYPASS-VALIDATION.md` 尚不存在，因此旁路证据尚未完成。

## 最终裁决

无 P0；三项源码级 P1 均已修复并由 76 个测试、fmt、clippy、diff check 通过验证。本轮评审为 **conditional**：仅剩旁路 mock/clawbot 质量门未完成，待 `t_762ec998` 报告到达后复核实际多轮/tool continuation/streaming/cache 与错误分类，再决定是否 approved。
