# Kimi K3 Cache Optimization — Context Recovery（上下文恢复 · 终版）

> **requested_model=deepseek-v4-flash** · provider=`eswitch` · api_mode=`chat`（平台声明）· **MODEL_IDENTITY_UNVERIFIED**（沙箱内无法独立证明底层模型，仅记录 harness 声明）。
> **文档性质：仅上下文恢复文档。未修改任何 Rust 源码、未修改任何生产配置、未部署。**
> 不含密钥 / Authorization 头 / 完整 prompt / 完整 tool schema / 完整 reasoning 原文。
> 供 Leader 或新会话**只读恢复**后按 `KIMI-K3-CACHE-OPTIMIZATION-FINAL-PLAN.md`（**唯一开发主入口**）派发实现。

---

## 1. 一句话

Phase 0 已 squash 合并至远程 master `31f9b851`；**Phase 1（3 commits）与 Phase 2a（`ae5a884`、`39c89b7`）已完成**，HEAD = `39c89b7`（5 commits ahead，**未 push**）；**下一步 = Phase 2b.1**（按修订后 FINAL-PLAN §6 Phase 2b 派发）；**Phase 2b 全门绿（G5/G6）后才进入 Phase 3**。**最终方案文档 = `KIMI-K3-CACHE-OPTIMIZATION-FINAL-PLAN.md`**（本文件与旧 PLAN 为辅助/历史）。**未 commit/push、未部署**；生产替换需用户另行授权。生产 0% 根因维持 **CONDITIONAL**（需 G2 升格）。

## 2. 仓库与远程基线

- 仓库：`/root/projects/codewhale-proxy/source`（remote `origin` = `https://github.com/zwczwczwc/cc-proxy.git`）。
- **远程 master = `31f9b851308d2845b69d35880e35e1805b8e4f18`**（PR #4 squash merge，tree `6e1a2132`；`git ls-remote` 实测 = 本地 ref）。
- 本地 `master@f6425e8` 是过期历史（内容已入 PR），**禁止用作「最新」依据**；历史审计分支 `chore/land-existing-cc-proxy-fixes @ 58f006b`（tree==master，上游已删）**已不再 HEAD**——当前 HEAD = feature 分支 `39c89b7`（见 §3）。

## 3. 当前 feature 分支

- `feat/kimi-k3-cache-optimization` @ `39c89b7`（基于 origin/master，tracking origin/master；**ahead 5**：Phase 1 `beb194c`/`c4bc942`/`eceedd8` + Phase 2a `ae5a884`/`39c89b7`）。**未 push**（远程无此分支）；feature 分支上未提交项 = 0（Phase 1/2a 已全部 commit，Phase 2b 待新 commit）。

## 4. dirty / untracked 状态（恢复时务必复核）

- **tracked 改动 = 0**（无未提交 Rust 代码）。
- **untracked = 32 项**（31 个 `.md` + `tools/`），**必须保留、不得进入任何提交**。本任务相关 3 个：
  - `KIMI-K3-CACHE-OPTIMIZATION-FINAL-PLAN.md`（**主入口**，新增）
  - `KIMI-K3-CACHE-OPTIMIZATION-PLAN.md`（旧版，历史参考）
  - `KIMI-K3-CACHE-OPTIMIZATION-CONTEXT-RECOVERY.md`（本文件）
- stash：`stash@{0}` = WIP on reform/cc-proxy-kimi-k3（stale，**勿 pop/覆盖**）。

## 5. 证据与报告（只读，位于 `/tmp/shared/kimi-cache-hit-issue/deepseek-reports/`）

| 报告 | 用途 |
|---|---|
| `20-research-facts-matrix-*` | 事实/假设/已决策/未知（F1-F8/A1-A5/U1-U10） |
| `21-architecture-review-*` | 架构裁决（IR 收敛/双 encoder/schema 重定位/Phase 4 拆 4 子步/Usage.cached_tokens 新增/provider 名归一） |
| `22-acceptance-matrix-review-*` | 测试矩阵裁决（golden 门改 per-wire/T01-T38/B1-B9） |
| `18-difference-adjudication-*` | live vs master 二进制裁决（功能等价、build→commit 溯源不可证） |
| `19-single-source-governance-*` | 单一来源 + 发布/回滚 artifact 治理（MANIFEST/Hash 对比纪律） |
| `08-phase0-land-*-rerun` | Phase 0 落地 + PR #4 合并证据 |
| `10-feature-baseline-prep-*` | feature 分支基线 |
| `04-kimi-direct-cache-probe-recheck` | 直连探针原始结论（C2a/C2b 对照、负结果、后端路由=CONDITIONAL） |
| `07-leader-evidence-review-*` | 门控 G1-G5 |
| `06-generic-cache-architecture-recheck` | 架构初稿 |
| `34-oss-grounded-final-adjudication-*` | OSS 证据最终裁决（三参考项目直接/间接/历史来源边界；GAP-A/GAP-B；A/B 硬门） |
| `43-phase2-spec-review-*` / `44-phase2-quality-review-*` | Phase 2 双 review（S1/S2/S3；MUST_FIX #1/#2；SHOULD_FIX #1-#4；138 tests 复核） |
| `45-phase2-scope-reconciliation-*` | **Phase 2a/2b/3 拆分裁决**（2b 四 commit 蓝图、Legacy vs Raw、NO-GO/回滚边界） |
| `46-phase2-scope-doc-amend-*` | 本修订记录（2a done / 2b pending、上下文入口） |
- 探针原始数据：`/tmp/kimi-cache-probe/results.json`、`probe.py`。

## 6. 首轮只读命令（新会话恢复第一步）

```bash
cd /root/projects/codewhale-proxy/source
git status --short --branch
git rev-parse HEAD origin/master
git ls-remote origin refs/heads/master     # 与 origin/master ref 比对
git branch -vv                             # 确认 feat/kimi-k3-cache-optimization @ 39c89b7（ahead 5，未 push）
git stash list                             # stash@{0} 勿 pop/覆盖
git status --porcelain | grep -c '^??'     # 预期 32（31 旧 + FINAL-PLAN）
git diff --stat                            # 预期空（tracked=0）
cargo test --locked --all-targets          # 基线 138 passed（可选，只写 gitignore 的 target/）
```

若 `origin/master` 与远程不一致 → 仅允许 `git fetch --no-tags origin master`（refs-only）；严禁 pull/rebase/reset/checkout 覆盖未跟踪文件。

## 7. 生产边界（恢复/继续时必须遵守）

- 不触碰：`/etc/cc-proxy`、systemd、`/usr/local/bin/cc-proxy`、**11441**、生产 config、clawbot 节点实例（11435）。
- 验证隔离：受控探针仅 `clawbot:11434`（synthetic 内容）；staging proxy 用 **11449** 独立端口 + 独立 config；验证结束无残留进程。
- 未 commit/push/merge（除非 Phase 授权）；未 reset/clean；未调用 11441 业务 API。
- 未跟踪 .md/tools 不得入 commit。

## 8. 下一步（按修订后 FINAL-PLAN §6 派发）

1. **Phase 1 与 Phase 2a 已完成**（HEAD `39c89b7`，ahead 5 未 push）：Phase 1 = IR 迁移（`beb194c`/`c4bc942`/`eceedd8`）；Phase 2a = additive foundation（`ae5a884`/`39c89b7`，`cargo test --locked --all-targets` = **138 passed / 0 failed**）。**范围裁决已落文档**：报告 43/44 发现的缺口正式重分类为 **Phase 2b**（**不是已完成项**）。
2. **下一步 = Phase 2b.1**（4 个零行为 integration commit 之一，见 FINAL-PLAN §6 Phase 2b）：`ProviderConfig.cache_policy: Option<CachePolicy>` + `CachePolicy`/`UsagePolicy` 类型 + validate hook（**不含 effort fail-fast**）+ 全部字面量补 `cache_policy: None` + default-off/旧 config 兼容测试。随后 2b.2（`prompt_cache_key` 字段 + 纯 `session_key()` + T16-T19 fail-closed；**不注入**；**Responses 不加字段**）、2b.3（Responses view/legacy 分离 + 流/非流测试）、2b.4（Chat view/legacy 分离 + `routes/messages.rs` policy 下传 + Chat 流/非流测试；**legacy wire/log 不变**）。
3. **Phase 2b 全门绿（G5/G6，每 commit golden + 全量测试）后才进入 Phase 3**（行为激活：provider canonicalization/upstream、effort fail-fast、key 注入、config.toml opt-in）；Phase 3 仍受 G3/G4/G7 门控。**G2 / 真实 A-B 不是 Phase 2b 前置**（离线/只读并行证据，门控激活与发布门 B8）。
4. **未 push、未部署**：feature 分支 ahead 5 未 push；未触碰 11441 / systemd / `/etc/cc-proxy` / `/usr/local/bin/cc-proxy`。**生产替换需用户另行授权**（11449 staging A/B 后、Leader 显式授权才可部署）。
5. **门控状态**：G1 已解除；G5/G6 在 Phase 2b 每 commit 复核；G3（live config）/G4（key 契约，§3.3 已定，注入在 Phase 3 fail-closed）未完成；G7 全程保持。
6. **每 Phase** 独立 commit、独立测试、独立 reviewer；NO-GO → revert 该 Phase 不发布。
7. **详细计划/测试矩阵/发布回滚/停止条件**：见 `KIMI-K3-CACHE-OPTIMIZATION-FINAL-PLAN.md` §6-§9（Phase 2 已修订为 2a/2b）。

## 附：编制报告

本恢复文档与终版方案由报告 `23-final-plan-draft-deepseek-v4-flash.md` 记录编制过程与回读验证。
