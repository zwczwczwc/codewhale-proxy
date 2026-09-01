# Responses Cache / Continuation A/B 评审报告

- 评审对象：`RESPONSES-CACHE-CONTINUATION-AB-COMPARISON.md`、`tools/responses_cache_ab.py`、`RESPONSES-CACHE-AB-PREREQUISITE-AUDIT.md`
- 评审范围：安全 dry-run、A/B 定义一致性、真实流量 blocker、脱敏边界与自动复核
- 评审限制：未执行真实 A/B，未发送业务 POST，未启动临时代理，未触碰生产业务端口 `11441`

## 结论

**verdict：conditional（CONDITIONAL_BLOCKER）**

已修复此前 P1/P2：脚本降级为明确的未完成安全骨架；`--run-real` 返回 `NOT_IMPLEMENTED` 并且不读取凭证、不发网络请求；A/B 定义与报告统一，前置审计中的相反旧标签已明确标为废弃并更正；`--self-test` 已精确断言完整安全契约（网络、secret、进程、URL、允许/禁止端口及 11441 业务请求）。真实 A/B、cache、continuation、SSE 和 keep-alive 仍未执行，不能据此宣称业务验收完成。

## 修复记录

1. `tools/responses_cache_ab.py` 不再执行部分 A warmup；未完整实现的真实模式统一阻断。
2. A 固定为直连 `clawbot:11434/v1/responses`，B 固定为 `127.0.0.1:11449/v1/messages`，报告同步更新；前置审计相反的历史标签已废弃并更正；两者当前均未执行。
3. dry-run 固定模型、输入和目标摘要，不读取环境变量；`--self-test` 精确验证 hash 稳定、场景为空、标签正确和完整安全契约。
4. 不读取或打印敏感凭证；脚本不再包含网络调用路径，也不会启动进程。

## 已执行验证

- `python3 tools/responses_cache_ab.py`：exit 0；dry-run blocker，无网络 I/O。
- `python3 tools/responses_cache_ab.py --self-test`：exit 0；自动复核 PASS。
- `python3 tools/responses_cache_ab.py --run-real`：exit 2；`NOT_IMPLEMENTED`，无凭证读取、无网络请求。
- `python3 -m py_compile tools/responses_cache_ab.py`：exit 0。
- `git diff --check`：exit 0。

复审 findings：此前 P1（A/B 标签冲突、self-test 安全契约不完整）均已修复；因真实实验未执行，裁决仍保持 **conditional（CONDITIONAL_BLOCKER）**。

## 放行条件

真实执行前必须有隔离凭证的明确授权，并由 reviewer 复核完整 A/B 场景实现、临时旁路监听范围、上游目标和清理流程；未满足前保持 blocker。健康检查、GET 404、dry-run hash 均不能作为业务 A/B 或 cache 结果。
