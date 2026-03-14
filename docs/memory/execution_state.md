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

- 在 `memory-governance` 阶段继续收口 Phase 1 lifecycle 与治理消费边界，并把 governed retrieval / orchestration 的入口稳定下来。

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
- 已落地 lifecycle 切口：
  - governed metadata 现在可动态派生 `active` / `stale` / `expired` 三种 lifecycle state
  - `rolling` / `durable` / `archival` 分别具备明确的 `review_at` / `expires_at` 窗口
  - `memory_list` tool 与 memory API 列表支持 `lifecycle` 过滤，并返回 `lifecycle_state` / `review_at` / `expires_at` / `promotion_candidate`
  - `durable` 且 `kind in {preference, decision, constraint, profile, project_state}` 的记录会被标记为晋升到 `MEMORY.md` 的候选
  - prompt builder / wizard 已同步提示 agent 使用 lifecycle 字段判断旧记忆是否应复用或晋升
- 已落地 tag 过滤切口：
  - `memory_list` tool 支持 `tags` 过滤，命中规则为“记录需包含全部请求 tag”
  - `/api/memory/agents/:id/kv` 支持 `tags` 查询参数，兼容重复参数与逗号分隔输入
  - `openfang-types::memory` 新增共享 helper，统一 tag filter 规范化与匹配语义，供 tool/API/后续 retrieval 复用
- 已落地 legacy cleanup 切口：
  - `/api/memory/agents/:id/kv/cleanup` 新增显式 audit/apply 入口
  - cleanup plan 会识别 legacy bare key、orphan metadata sidecar、以及 canonical key 缺失 metadata 三类问题
  - `apply=true` 时会执行迁移 legacy bare key、删除 orphan sidecar、回填默认 governed metadata
  - cleanup 规划逻辑已下沉到 `openfang-types::memory::plan_memory_cleanup`
- 已落地 cleanup tool exposure 切口：
  - runtime builtin tool 新增 `memory_cleanup`，支持 `apply` / `limit` 参数，并直接消费共享 cleanup plan
  - `KernelHandle` 新增 shared memory delete bridge，供 tool 层执行 legacy key 删除、orphan sidecar 删除与 metadata 回填
  - prompt builder / setup wizard 已把 `memory_cleanup` 纳入 memory capability 指导，提示 agent 先 audit 再 apply
- 已落地 governed retrieval consumption 切口：
  - `openfang-types::memory` 新增 `select_governed_memory_prompt_candidates`，统一为 runtime / 后续 retrieval 选择 governed KV 候选
  - runtime 动态 memory context 现在会额外注入 `Governed memory candidates`，不再只依赖 semantic recall 与 `session_*` 摘要
  - governed 候选优先读取 kernel 暴露的 shared memory list，因此 memory tool / memory API 写入的 shared KV 也会进入动态 retrieval 消费路径
  - governed 候选当前会消费 `kind` / `tags` / `freshness` / `lifecycle_state` / `promotion_candidate`，并排除 `expired` 记录
  - governed 候选已具备最小 query-aware 排序：会用当前 user message 对 `key` / `tags` / `kind` / `value` 做轻量打分，再回落到治理优先级排序
- 已完成本轮验证：
  - `cargo build --workspace --lib`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings` 仅剩 `openfang-cli/src/main.rs` 既有 `collapsible_else_if`
  - live integration 验证通过：`/api/health`、memory KV PUT/GET/LIST/DELETE、namespace/prefix/include_internal/lifecycle 过滤、真实 `/api/agents/{id}/message`、`/api/budget`、`/api/budget/agents`、dashboard HTML
  - live tag 过滤验证通过：
    - `GET /api/memory/agents/assistant/kv?tags=profile&tags=ui&limit=10` 仅返回 `pref.tag_filter.theme`
    - `GET /api/memory/agents/assistant/kv?tags=project,alpha&limit=10` 仅返回 `project.tag_filter.note`
    - 真实 `assistant` message 后 `daily_spend` 从 `0.04491296` 增到 `0.045257900000000004`
  - live cleanup 验证通过：
    - `POST /api/memory/agents/assistant/kv/cleanup {"apply":false}` 返回 `migrate_legacy_key` / `backfill_metadata` / `delete_orphan_metadata` audit 结果
    - `POST /api/memory/agents/assistant/kv/cleanup {"apply":true}` 实际完成 legacy bare key 迁移、metadata 回填与 orphan sidecar 删除
    - cleanup 后 `GET /api/memory/agents/assistant/kv/cleanup_legacy_probe` 返回 `key=general.cleanup_legacy_probe` 且 `source=memory_cleanup_api`
    - cleanup 后 `project.cleanup_probe.note` 返回 `governed=true` 且 `source=memory_cleanup_api`
    - 真实 `assistant` message 后 `daily_spend` 从 `0.04600507` 增到 `0.04670917`
  - live lifecycle 验证结果：
    - `pref.lifecycle_test.theme` 返回 `lifecycle_state=active`、`review_at=2026-04-11T18:31:27.386977+00:00`、`expires_at=null`、`promotion_candidate=true`
    - `project.lifecycle_probe.note` 返回 `lifecycle_state=active`、`review_at=2026-03-19T18:31:27.389653+00:00`、`expires_at=2026-04-11T18:31:27.389653+00:00`、`promotion_candidate=false`
    - `include_internal=true` 仍可看到 `__openfang_schedules`
    - 真实 agent message 后 `daily_spend` 从 `0.39378700000000005` 增到 `0.4021756`
  - live governed retrieval 验证通过：
    - 通过 memory API 写入 `pref.retrieval_probe.theme` 与 `project.retrieval_probe.status` 两条 shared governed KV
    - 真实 `assistant` message 后，`~/.openfang/workspaces/assistant/logs/llm.log` 中出现 `Governed memory candidates` 区段
    - log 中明确包含：
      - `pref.retrieval_probe.theme (kind=preference, freshness=durable, lifecycle=active, tags=profile,ui,retrieval_probe, promotion_candidate)`
      - `project.retrieval_probe.status (kind=project_state, freshness=rolling, lifecycle=active, tags=project,retrieval_probe)`
    - 第二轮 live verification 中 `daily_spend` 从 `0.05442078` 增到 `0.05510341`
  - live query-aware governed retrieval 验证通过：
    - 注入了 5 条更高治理优先级的 durable `pref.query_probe.*` 与 1 条较低治理优先级的 rolling `project.alpha.query_probe.status`
    - 当用户消息是 `What is the alpha project status right now?` 时，`llm.log` 的 `Governed memory candidates` 前 4 中包含 `project.alpha.query_probe.status`
    - 同一份 log 里其它非 project 查询轮次仍由 durable `pref.query_probe.*` 占满前 4，说明 project probe 是被 query-aware 排序拉升，而不是静态治理顺序自然排到前面
    - 该轮真实 message 的回复直接使用了 probe 值 `Alpha launch blocked on QA signoff.`
    - 本轮 live verification 中 `daily_spend` 从 `0.05580307` 增到 `0.05715266`
  - live `memory_cleanup` tool 验证通过：
    - 临时创建了仅授予 `memory_cleanup` 的 MiniMax verifier agent，真实 `/api/agents/{id}/message` 返回成功
    - verifier 的 `llm.log` 中出现两次真实 tool 调用：先 `memory_cleanup {"apply":false}`，再在命中 `tool_cleanup_legacy_probe` / `project.tool_cleanup.status` / `pref.tool_cleanup_orphan` 后执行 `memory_cleanup {"apply":true}`
    - 该轮真实 message 的回复明确返回 `general.tool_cleanup_legacy_probe` 与 `project.tool_cleanup.status`，并确认 orphan metadata 已删除
    - `/api/budget` 中 `daily_spend` 从 `0.05844012` 增到 `0.06483442`，`/api/budget/agents` 中新增 verifier agent 花费 `0.0063943`
    - live 验证后已删除 probe，并把与本次 probe 无关的 shared legacy key 迁移恢复回原状

## 进行中

- 继续推进 Phase 1 后续切口：governed prompt candidates 的 query-aware 规则是否需要更强，以及 lifecycle snapshot 是否需要进入 dashboard 或更高层 orchestration。

## 下一步动作

- 评估 governed prompt candidates 的 query-aware 打分是否需要继续引入停用词、短语匹配、namespace/kind hint 或显式过滤。
- 评估 lifecycle snapshot 是否需要进入 dashboard 或 prompt orchestration 的更高层，而不只停留在 API/tool 响应中。
- 在切换电脑或结束一轮实质性工作前，持续更新本文件。

## 风险与阻塞

- `cargo clippy --workspace --all-targets -- -D warnings` 当前仍被 `openfang-cli/src/main.rs` 中既有问题阻塞；按仓库约束，本轮未修改 `openfang-cli`。
- 当前 embedding provider 本地端点 `http://localhost:11434/v1/embeddings` 离线，live LLM 调用期间会回退到 text search；这不阻塞本轮 KV governance 验证，但会影响 embedding recall 路径验证。
- 当前 `assistant` agent 的 gemini tool-call 路径仍存在既有 `thought_signature` 兼容问题；本轮 `memory_cleanup` tool live verification 因此改用临时 MiniMax verifier agent 完成。
- 如果后续启动工作时不先读取本文件，分支纪律和连续性可能重新漂移。

## 验证清单

- 恢复工作时先读取本文件。
- 读取 `## 设计文档` 中列出的全部文档。
- 编码前确认当前分支与 worktree 和本文件一致。
- 当阶段、分支、worktree 或下一步动作发生变化时，更新本文件。

## 最后更新时间

- 2026-03-14 Asia/Shanghai
