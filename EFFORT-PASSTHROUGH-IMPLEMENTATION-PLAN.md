# cc-proxy 思考等级透传 + 档位归一化 实施计划（2026-08-27）

## 目标
Claude Code 入站 effort（output_config.effort）透传到上游；上游无对应档位时按"档位组"归一化，而非死映射表。
用户已授权（本会话明确同意："cc-proxy需要支持思考等级透传"）。

## 设计：三层优先级 + tier 归一化

档位序（低→高）：off < minimal < low < medium < high < top(xhigh≡max)

effort 来源优先级：
1. pinned_effort（cache_policy 显式 pin，现网行为不变）
2. 入站 output_config.effort（新增读取；仅接受已知档位词，未知值忽略→回退默认）
3. 默认 "xhigh"（现状兜底）

映射算法 resolve_effort(inbound, provider)：
1. effort_map 直接命中 → 用之（现网逐字节不变）
2. 未命中 → 在上游支持集 support = effort_map.values() 中找同 tier 成员
   （tier(xhigh)=tier(max)=top；如入站 max → glm 支持 {low,medium,high,max} → max 透传）
3. 同 tier 无成员 → 按 RANK 向下降级到最近支持档（如 medium → deepseek{high,max} → high）
4. 全部低于最小支持档 → 取 support 中 RANK 最小者

## 改动面（文件级）

### 1. src/anthropic/types.rs
- MessagesRequest 加字段 `pub output_config: Option<Value>`（#[serde(default)]，宽松 Value 不建强类型，
  只取 .effort 字符串）。29 处 struct literal 测试夹具需同步补字段（或用 ..Default::default？不行，无 Default；
  逐个补 `output_config: None`）。

### 2. src/reasoning/apply_effort.rs（新函数）
- `pub fn effort_tier(e: &str) -> Option<u8>`：off=0, minimal=1, low=2, medium=3, high=4, xhigh/max=5
- `pub fn resolve_effort(inbound: &str, provider: &ProviderConfig) -> String`：上述三层算法
- apply_reasoning_effort 的 unknown 分支改调 resolve_effort（替换 unwrap_or("high") 硬编码）

### 3. src/anthropic/converter.rs
- L289/L303 两处 `pinned_effort.unwrap_or("xhigh")` → 三态来源：
  pinned > inbound(output_config.effort) > "xhigh"
- 新 helper `fn inbound_effort(req: &MessagesRequest) -> Option<String>`：
  req.output_config.as_ref()?.get("effort")?.as_str() 校验 ∈ {off,minimal,low,medium,high,xhigh,max}
- apply_effort_direct 内 unknown 分支同样走 resolve_effort

### 4. src/responses/request.rs（GPT Responses 路径）
- L149-152 固定取 effort_map["max"] → 改为同三态来源 + resolve_effort
- 注意 gpt effort_map xhigh→xhigh、max→max 均在支持集，行为兼容

### 5. 测试夹具同步
- grep "MessagesRequest {" 全部 29 处补 output_config: None
- ProviderConfig 夹具不受影响（未加字段）

### 6. config.toml（生产，部署阶段才动）
- 无需新字段！support 集从 effort_map.values() 自动推导。

## 兼容性保证
- 现网所有请求（CC 交互模式发 adaptive+output_config.effort=xhigh）：入站 xhigh → deepseek map xhigh→max（命中第1层，字节不变）；glm 同。
- headless -p 模式（无 thinking 字段）：is_reasoning_model 分支默认仍 xhigh→max，不变。
- kimi pinned high：pin 优先级最高，不变。
- gpt tools 强制 off：分支保留。

## TDD 门（cc-proxy-ops 规范）
cargo fmt --check && cargo check --locked && cargo test --locked --all-targets && cargo clippy --locked --all-targets --all-features -- -D warnings

## 验收（隔离旁路，不触生产）
- 127.0.0.1:11449 → http://clawbot:11434，Bearer not-needed
- A: 入站 output_config.effort=xhigh + thinking adaptive → 上游 reasoning_effort=max（deepseek/glm）
- B: 入站 effort=medium → deepseek 归一化为 high；glm 透传 medium
- C: 入站 effort=max → glm 透传 max；deepseek map 命中 max→max
- D: 无 output_config → 行为与现网一致（xhigh→max）
- E: thinking disabled → thinking.type=disabled + reasoning_effort=None（回归不破坏）
- F: glm-5.3-flash disabled 边界：入站关思考 → 上游 400 1210 已知边界（记录，不在本次修）
- 结束确认 11449 无监听、生产 11441 健康。

## 部署边界
- 本计划只改源码工作树 + 本地验证。生产替换（stop→cp→start + journal 回归）须用户单独授权后执行。
