# Responses / Chat 隔离回归证据

日期：2026-08-06  
分支：`feat/gpt-responses-transport`

## 路由契约

- `gpt-5.6-luna` 及 `claude-sonnet-4-6` alias 的 model profile 显式设置
  `wire_api = "responses"`，因此 `/v1/messages` 到上游使用
  `/v1/responses`。
- DeepSeek、GLM、Kimi profile 未设置 Responses wire，`WireApi` 默认值为
  `chat_completions`，继续使用 `/v1/chat/completions`。
- Responses 分支在 `src/routes/messages.rs` 中直接返回上游/转换错误；该分支
  没有调用 Chat client，因此失败不会静默 fallback 到 Chat。

## 自动化证据

`src/config.rs` 的 `wire_api_is_responses_only_for_gpt_profile_and_chat_by_default`
覆盖了 gpt canonical name、gpt alias、DeepSeek alias、GLM、Kimi 以及未知模型
默认 Chat 的 wire 选择。Responses request 单测另外覆盖了 assistant
`output_text`、tool continuation item、synthetic tail、三层 prefix hash 和
多轮公共历史稳定性。

完整质量门以本次任务实际运行结果为准：

```text
cargo test --all-targets --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

本地质量门不代表真实上游 cache 命中或业务 E2E 已通过；此前旁路报告中的
502、timeout 和 cache miss 按错误分类单独记录，不应被改写成 cache 结论。