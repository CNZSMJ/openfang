# 技术方案：Memory Governance Phase 1

## 1. 文档定位

本文档描述的是从 `custom/0.1.0` 基线切出 `memory-governance` 阶段后，第一批实际落地的治理规则与后续推进顺序。

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

## 4. 当前不做的事情

本阶段第一批实现明确不做：

1. 不改 `kv_store` 表结构。
2. 不引入 TTL、垃圾回收或自动清理仲裁。
3. 不引入 tag 索引或 semantic / hybrid retrieval。
4. 不把治理规则继续散落到 prompt runtime 之外的多套实现中。
5. 不做一次性全量 legacy bare key 迁移。

## 5. 代码落点

- `crates/openfang-types/src/memory.rs`
  - memory key 规范化、namespace 提取、prefix 匹配、兼容 lookup helper
- `crates/openfang-runtime/src/tool_runner.rs`
  - tool 输入规范化
  - metadata sidecar 写入
  - `memory_list` 默认隐藏内部 key，并返回 governed 字段
- `crates/openfang-runtime/src/prompt_builder.rs`
  - prompt 协议补充 namespaced key 约束
- `crates/openfang-kernel/src/wizard.rs`
  - setup hint 补充 namespaced key 指导
- `crates/openfang-api/src/routes.rs`
  - memory API 对齐治理规则
  - API 列表过滤与 governed metadata 折叠

## 6. 下一步建议

在当前切口稳定后，下一步按以下顺序推进：

1. 引入 lifecycle 策略：过期、降级、晋升到 `MEMORY.md` 的标准。
2. 评估是否增加显式 cleanup / migrate 工具，消化 legacy bare key 和陈旧 governed 记录。
3. 为 `memory_list` 增加更强的 tag 过滤能力。
4. 给后续 embedding / hybrid retrieval 预留对 `kind` / `tags` / `freshness` 的消费接口。
