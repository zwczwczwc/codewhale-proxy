# Responses Cache / Continuation A/B 对照报告

- 时间：2026-08-06
- 状态：`CONDITIONAL_BLOCKER`
- 范围：仅完成确定性、脱敏 dry-run 骨架与自动无网络断言；未发送业务 POST，未修改 Rust 源码或生产配置。
- 脚本：`tools/responses_cache_ab.py`

## 安全结论与 blocker

当前脚本是**未完成的安全骨架**，不是 A/B 执行器。默认 dry-run 使用固定模型和固定 synthetic 输入，不读取环境变量中的凭证或模型配置，不进行网络/进程 I/O。`--run-real` 明确返回 `NOT_IMPLEMENTED`（exit code 2），不会读取凭证，也不会发送请求；因此不能被误认为完成真实 A/B。

负责人尚未提供经批准的隔离测试凭证和完整旁路条件。真实流量保持 blocker；本次没有执行真实 A/B、cache hit/miss、tool continuation、SSE、keep-alive 或 timeout 对照。所有“未执行”均不计为 cache miss、协议失败或业务成功。

## 固定 A/B 定义

与任务原始定义统一为（前置审计第 6 节相反的历史标签已明确废弃并更正）：

- **A：** 直连 `http://clawbot:11434/v1/responses`。
- **B：** `http://127.0.0.1:11449/v1/messages` Anthropic 临时旁路。

当前 A、B 均未执行。dry-run 仅展示上述目标定义，不连接任何目标；尤其不使用 `127.0.0.1:11434`，不向生产业务端口 `11441` 发送请求。

## 已执行命令与结果

| 命令 | exit code | 脱敏结果 |
|---|---:|---|
| `python3 tools/responses_cache_ab.py` | 0 | `mode=dry-run`, `status=BLOCKED_CONDITIONAL`；无网络/凭证读取 |
| `python3 tools/responses_cache_ab.py --self-test` | 0 | `mode=self-test`, `status=PASS`；固定 hash、A/B 标签和无网络契约通过 |
| `python3 tools/responses_cache_ab.py --run-real` | 2 | `status=NOT_IMPLEMENTED`；未读取凭证、未发送请求 |
| `python3 -m py_compile tools/responses_cache_ab.py` | 0 | Python 语法通过 |
| `git diff --check` | 0 | 无 whitespace 错误 |

## Dry-run 可复核边界

`--self-test` 自动精确验证：

- `executed_scenarios` 为空；
- A/B 标签固定为直连 Responses 与 `127.0.0.1:11449` Anthropic；
- 模型固定为 `gpt-5.6-luna`，不读取 `RESPONSES_AB_MODEL` 等环境变量；
- 连续 dry-run 的 request hash 相同，环境配置不会改变 hash；
- dry-run 的安全契约包含“不读取凭证”和“不进行网络 I/O”。
- 不使用 `127.0.0.1:11434`，不向 `11441` 发业务请求，不启动进程，不打印 secret；
- 允许端口仅为上游 `11434` 与临时本地 `11449`，禁止本地 `11434`、生产 `11441`，且允许/禁止 URL 集合正确；
- `--run-real` 的状态为 `NOT_IMPLEMENTED`，不读取凭证、不发网络请求。

1. `--self-test` 精确断言无网络调用、不读取/打印 secret、不使用 `127.0.0.1:11434`、不向 `11441` 发业务请求、不启动进程，以及 A/B URL、允许/禁止端口集合正确。
2. `--run-real` 返回 `NOT_IMPLEMENTED`，并由 self-test 断言其无网络调用、无凭证读取。

脚本输出只包含长度、截断 hash、固定目标定义和场景计划，不输出 prompt、tool schema、Authorization、响应正文或异常正文。当前示例 hash 不是业务结果，也不证明 cache 命中/未命中。

## 场景状态

| 组 | 目标 | warmup | history | function-call continuation | SSE/keep-alive | 结论 |
|---|---|---|---|---|---|---|
| A | 直连 `clawbot:11434/v1/responses` | 未执行 | 未执行 | 未执行 | 未执行 | blocker |
| B | `127.0.0.1:11449/v1/messages` | 未执行 | 未执行 | 未执行 | 未执行 | blocker |

## 后续放行条件

只有在完整实现并经 reviewer 复核后，才可考虑真实执行：隔离凭证受控注入、临时旁路仅监听 `127.0.0.1:11449` 且仅指向 `clawbot:11434`、A/B 全场景独立记录 HTTP 错误/timeout/cache/continuation，结束后确认旁路退出且不触碰 `11441`。在此之前不得启用真实流量路径。

## 文件变更

- 更新 `tools/responses_cache_ab.py`：安全骨架、确定性 dry-run、`--self-test`、`--run-real` NOT_IMPLEMENTED blocker。
- 更新本报告；真实 A/B 仍明确为未执行。
- 未修改 Rust 源码、`config.toml` 或生产服务。
