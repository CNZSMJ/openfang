# 记忆计划执行状态

## 约定

- 该文件是持续推进记忆计划的唯一执行入口。
- 助手在开始任何新的记忆计划任务前，必须先读取该文件。
- 未经用户明确批准，助手不得重命名、删除、重排本文件中的章节。
- 助手可以在正常工作过程中更新各章节中的字段内容。

## 当前阶段

- `phase-1-memory-governance`

## 基线分支

- `custom/0.1.0`

## 当前工作分支

- `memory-governance`

## 当前 worktree 路径

- `/Users/huangjiahao/workspace/openfang-0.1.0/memory-governance`

## 设计文档

- `docs/memory/agent_memory_enhancement_plan.md`
- `docs/memory/memory_governance_plan.md`

## 当前目标

- 在 `memory-governance` 阶段继续收口 Phase 1 第二批治理实现：memory record schema、写入准入、冲突处理与 API/tool 对齐。

## 已完成

- 已重写记忆增强设计文档，使其与当前分支中的实际实现架构一致。
- 已明确 `MEMORY.md` 指的是 agent workspace identity file，而不是仓库中的任意 `MEMORY.md`。
- 已确认后续阶段交付顺序：memory governance、embedding/hybrid retrieval、prompt architecture、assistant memory autoconverge。
- 已确认后续记忆计划管理文档统一放在 `docs/memory/` 下。
- 已将 `feature/enhance-memory-recall-and-store` 合并回 `custom/0.1.0`，形成 Phase 0 基线。
- 已完成一次合并后验证：`cargo build --workspace --lib`、`cargo test --workspace`、最小 live integration 成功。
- 已从 `custom/0.1.0` 切出 `memory-governance` 分支，并创建独立 worktree。
- 已新增 `docs/memory/memory_governance_plan.md`，明确 Phase 1 的治理边界和下一步顺序。
- 已落地第一批治理实现：
  - bare key 自动规范为 `general.<key>`
  - `memory_recall` / memory API 优先命中 canonical key，再向后兼容 legacy bare key
  - `memory_list` 默认隐藏 internal keys，并返回 `namespace` / `internal` 元数据
  - `/api/memory/agents/:id/kv/:key` 的 PUT/GET/DELETE 与 tool 层规则对齐
- 已落地第二批治理实现：
  - 共享 KV 记录新增 sidecar metadata schema：`namespace` / `kind` / `tags` / `freshness` / `source` / `updated_at`
  - `memory_store` 与 memory API PUT 支持 `kind` / `tags` / `freshness` / `conflict_policy`
  - 用户侧写入显式拒绝保留 internal key；`skip_if_exists` 会同时检查 canonical key 与 legacy bare key
  - `memory_list` 与 memory API 列表默认隐藏 metadata sidecar，并返回 `governed` / `kind` / `tags` / `freshness` / `source` / `updated_at`
  - memory API 列表新增 `namespace` / `prefix` / `include_internal` / `limit` 过滤入口
- 已完成本轮验证：
  - `cargo build --workspace --lib`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings` 仅剩 `openfang-cli/src/main.rs` 既有 `collapsible_else_if`
  - live integration 验证通过：`/api/health`、memory KV PUT/GET/LIST/DELETE、namespace/prefix/include_internal 过滤、真实 `/api/agents/{id}/message`、`/api/budget`、`/api/budget/agents`、dashboard HTML

## 进行中

- 继续推进 Phase 1 后续切口：lifecycle、legacy 清理策略、以及治理元数据在后续检索路径中的消费方式。

## 下一步动作

- 在当前 branch 上定义 memory lifecycle：过期、降级、晋升到 `MEMORY.md` 的标准。
- 评估是否需要为 legacy bare key 和 governed sidecar 做一次后台迁移或清理工具，避免长期双写遗留。
- 明确后续 embedding / hybrid retrieval 如何消费 `kind` / `tags` / `freshness` 等治理字段。
- 在切换电脑或结束一轮实质性工作前，持续更新本文件。

## 风险与阻塞

- `cargo clippy --workspace --all-targets -- -D warnings` 当前仍被 `openfang-cli/src/main.rs` 中既有问题阻塞；按仓库约束，本轮未修改 `openfang-cli`。
- 当前 embedding provider 本地端点 `http://localhost:11434/v1/embeddings` 离线，live LLM 调用期间会回退到 text search；这不阻塞本轮 KV governance 验证，但会影响 embedding recall 路径验证。
- 如果后续启动工作时不先读取本文件，分支纪律和连续性可能重新漂移。

## 验证清单

- 恢复工作时先读取本文件。
- 读取 `## 设计文档` 中列出的全部文档。
- 编码前确认当前分支与 worktree 和本文件一致。
- 当阶段、分支、worktree 或下一步动作发生变化时，更新本文件。

## 最后更新时间

- 2026-03-13 Asia/Shanghai
