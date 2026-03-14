# 技术方案：Memory Governance Phase 1

## 1. 文档定位

本文档描述的是从 `custom/0.1.0` 基线切出 `memory-governance` 阶段后，当前已经落地的治理规则与后续推进顺序。

它聚焦的是共享 KV memory 的治理边界，不覆盖 embedding / hybrid retrieval，也不尝试一次性解决完整 prompt attention architecture。

## 2. 阶段目标

当前阶段优先解决三个问题：

1. 让用户侧 `memory_store` / `memory_recall` / `memory_list` 脱离“任意 key 任意写”的无序状态。
2. 在不改底层 SQLite schema 的前提下，为后续 schema、tag、TTL 和清理策略预留稳定入口。
3. 降低 `memory_list` 被内部系统 key 污染的风险。

## 3. 已落地规则

### 3.1 用户 memory key namespacing

- 用户侧 bare key 会被规范成 `general.<key>`。
- 已经 namespaced 的 key 继续保留，例如 `project.alpha.decision`。
- 内部系统 key 继续保留原样，例如：
  - `session_*`
  - `__openfang_*`

### 3.2 Recall / delete 向后兼容

- 当用户请求 bare key 时，系统会优先查规范化后的 `general.<key>`，再回退到 legacy bare key。
- 这保证旧数据和新规则可以共存，不需要一次性迁移整个 KV 池。

### 3.3 `memory_list` 默认隐藏内部 key

- `memory_list` 默认不返回内部系统 key。
- 仅在显式 `include_internal=true` 时，才暴露 `session_*` 或 `__openfang_*`。
- 返回结果补充：
  - `namespace`
  - `internal`

### 3.4 API 与 tool 行为对齐

- `/api/memory/agents/:id/kv/:key` 的读写删逻辑与 tool 层共用同一套 key 规范。
- API 列表结果同样补充 `namespace` 与 `internal`。

### 3.5 Governed record metadata sidecar

- 用户侧 KV 记录现在会伴随一条 internal sidecar metadata：
  - `__openfang_memory_meta.<canonical_key>`
- metadata 当前字段包括：
  - `schema_version`
  - `key`
  - `namespace`
  - `kind`
  - `tags`
  - `freshness`
  - `source`
  - `updated_at`
- sidecar 只用于治理元数据，不改变底层 `kv_store` schema，也不改变原 value 的读取格式。
- `memory_list` 和 memory API 默认不暴露 sidecar 记录，而是把 governance 字段折叠回主记录响应。

### 3.6 写入准入与冲突处理

- `memory_store` / memory API PUT 现在拒绝写入保留 internal key，例如：
  - `session_*`
  - `__openfang_*`
- 用户写入可选携带：
  - `kind`
  - `tags`
  - `freshness`
  - `conflict_policy`
- `conflict_policy=skip_if_exists` 会检查 canonical key 和 legacy bare key，避免在旧数据存在时再写出一份新的 canonical 记录。
- API 列表增加了与 tool 层一致的过滤入口：
  - `namespace`
  - `prefix`
  - `include_internal`
  - `limit`

### 3.7 Lifecycle 快照与晋升候选

- governed 记录现在会基于 metadata 计算 lifecycle snapshot，不改底层 `kv_store` 结构，也不额外持久化一份 lifecycle 状态。
- 当前窗口规则为：
  - `freshness=rolling`
    - `review_at = updated_at + 7 days`
    - `expires_at = updated_at + 30 days`
  - `freshness=durable`
    - `review_at = updated_at + 30 days`
    - `expires_at = null`
  - `freshness=archival`
    - `review_at = updated_at + 180 days`
    - `expires_at = null`
- lifecycle state 由读取时刻动态计算：
  - 到达 `review_at` 之前为 `active`
  - 到达 `review_at` 之后、且未到 `expires_at` 时为 `stale`
  - 到达 `expires_at` 之后为 `expired`
- 当前晋升到 agent workspace `MEMORY.md` 的候选标准只做“可观测提示”，不做自动写入：
  - `freshness=durable`
  - `kind` 属于 `preference` / `decision` / `constraint` / `profile` / `project_state`
- `memory_list` tool 与 `/api/memory/agents/:id/kv` 现在都支持 `lifecycle=active|stale|expired` 过滤，并在响应中返回：
  - `lifecycle_state`
  - `review_at`
  - `expires_at`
  - `promotion_candidate`
- 单条读取 `/api/memory/agents/:id/kv/:key` 也会返回同样的 lifecycle 字段，方便 UI 或后续 retrieval 直接消费。

### 3.8 Tag 过滤与治理消费边界

- `memory_list` tool 现在支持 `tags` 过滤；只有包含全部请求 tag 的 governed 记录才会返回。
- `/api/memory/agents/:id/kv` 同样支持 `tags` 查询参数，可通过重复参数或逗号分隔形式传入多个 tag。
- `openfang-types::memory` 新增共享 helper，用于：
  - 规范化 tag 过滤输入
  - 判断 governed metadata 是否满足 tag 过滤
- 这样 tool、API 与后续 retrieval 消费方可以复用同一套治理过滤语义，而不是各自实现一遍。

### 3.9 Legacy Cleanup Audit / Apply

- `/api/memory/agents/:id/kv/cleanup` 现在提供显式治理清理入口，支持：
  - `apply=false`：只返回 audit 结果，不改数据
  - `apply=true`：按治理规则执行修复
- 当前 cleanup plan 会识别三类问题：
  - legacy bare key：迁移到 canonical `general.<key>`，或在 canonical 已存在时删除重复 bare key
  - orphan metadata sidecar：删除缺失主记录的 sidecar
  - missing metadata：为 canonical 用户 key 回填默认 governed metadata
- cleanup 回填 metadata 时，默认使用：
  - `kind=fact`
  - `freshness=durable`
  - `source=memory_cleanup_api`
- cleanup 规划逻辑已收敛到 `openfang-types::memory::plan_memory_cleanup`，避免 API 层再次内联一套规则。

## 4. 当前不做的事情

本阶段当前实现明确不做：

1. 不改 `kv_store` 表结构。
2. 不引入自动 TTL 删除、垃圾回收或后台清理仲裁。
3. 不引入 tag 索引或 semantic / hybrid retrieval。
4. 不把治理规则继续散落到 prompt runtime 之外的多套实现中。
5. 不做一次性全量 legacy bare key 迁移。

## 5. 代码落点

- `crates/openfang-types/src/memory.rs`
  - memory key 规范化、namespace 提取、prefix/tag 匹配、兼容 lookup helper
  - cleanup audit plan（legacy key / orphan metadata / missing metadata）
- `crates/openfang-runtime/src/tool_runner.rs`
  - tool 输入规范化
  - metadata sidecar 写入
  - `memory_list` 默认隐藏内部 key，并返回 governed + lifecycle 字段
  - `memory_list` tags 过滤
- `crates/openfang-runtime/src/prompt_builder.rs`
  - prompt 协议补充 namespaced key 约束，以及 tags/lifecycle 使用提示
- `crates/openfang-kernel/src/wizard.rs`
  - setup hint 补充 namespaced key、tags 与 lifecycle 指导
- `crates/openfang-api/src/routes.rs`
  - memory API 对齐治理规则
  - API 列表过滤、governed metadata 折叠与 lifecycle 返回
  - API 列表 tags 过滤
  - cleanup audit/apply endpoint

## 6. 下一步建议

在当前切口稳定后，下一步按以下顺序推进：

1. 评估是否需要把 cleanup audit/apply 进一步暴露给 tool 层或 dashboard，而不只停留在 API。
2. 评估是否需要在 dashboard / higher-level orchestration 中直接暴露 tag + lifecycle snapshot，而不只停留在 tool/API 层。
3. 给后续 embedding / hybrid retrieval 预留对 `kind` / `tags` / `freshness` / `lifecycle_state` 的消费接口。
4. 决定 lifecycle snapshot 是否需要进入更高层 prompt orchestration，而不仅仅停留在当前提示文案与 API/tool 可见层。
