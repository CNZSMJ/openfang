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

### 3.10 Governance Metadata Consumption in Dynamic Retrieval

- runtime 的动态 memory context 现在会额外注入一组 governed KV 候选，而不只依赖 semantic recalled fragments 与 `session_*` 摘要。
- governed 候选优先读取 kernel 暴露的共享 memory list，因此会消费 memory tool / memory API 写入的 shared KV，而不是只停留在 agent 私有 structured store。
- governed 候选的筛选逻辑已下沉到 `openfang-types::memory::select_governed_memory_prompt_candidates`，供后续 embedding / hybrid retrieval 继续复用。
- 当前选择策略会：
  - 排除 internal key、metadata sidecar 与 `expired` lifecycle 记录
  - 优先考虑 `preference` / `decision` / `constraint` / `profile` / `project_state`
  - 允许带 tag 的 governed 记录进入候选集
  - 先构建共享 query profile：stopword 清理、2/3-gram phrase 提取，以及 namespace / kind hint
  - 再根据该 profile 对 `key` / `tags` / `kind` / `value` 做加权 query-aware 打分，然后按 lifecycle、promotion candidate、是否带 tags、freshness、updated_at 排序
- prompt 注入层当前会把这些候选以独立的 `Governed memory candidates` 小节附加到动态 memory context 中，并显式暴露：
  - `kind`
  - `freshness`
  - `lifecycle`
  - `tags`
  - `promotion_candidate`
- live verification 已确认 shared KV probe 会真实进入 assistant workspace 的 `logs/llm.log`，证明该路径不是死代码。
- live verification 还确认了 query-aware 排序不是空转：在 5 条 durable preference 与 1 条 rolling project probe 并存时，project probe 只会在 project-status 查询里被拉升到前 4。
- 这一步仍然不是完整的 query-aware hybrid retrieval，但它已经让 governed prompt candidates 具备最小问题感知能力，后续不必再从零设计一次字段语义。

### 3.11 Cleanup 能力进入 Tool Layer

- runtime builtin tools 现在新增 `memory_cleanup`，作为 shared governance cleanup 的 tool 层入口。
- `memory_cleanup` 当前输入支持：
  - `apply`
  - `limit`
- tool 返回 audit/apply 合并结果，包括：
  - `planned`
  - `counts`
  - `applied_counts`
  - `findings`
- 为了让 tool 层真正执行 cleanup，而不只是复用 API 返回，`KernelHandle` 新增了 shared memory delete bridge。
- 这样 cleanup 规则不再只停留在 `/api/memory/agents/:id/kv/cleanup`，agent 自己也可以：
  - 先 audit
  - 判断是否命中 legacy bare key / orphan sidecar / missing metadata
  - 再决定是否 apply
- prompt builder 的 memory guidance 与 wizard 的 memory capability 提示也已同步加入：
  - 当 `memory_list` 暴露出治理异常时，先用 `memory_cleanup` audit
  - 再在确有必要时 apply
- live verification 已确认这不是“定义了 tool 但 agent 不会用”的死代码：
  - 临时 MiniMax verifier agent 在真实 `/api/agents/{id}/message` 中先调用 `memory_cleanup {"apply":false}`
  - 命中 `tool_cleanup_legacy_probe` / `project.tool_cleanup.status` / `pref.tool_cleanup_orphan` 后，再调用 `memory_cleanup {"apply":true}`
  - `llm.log` 中有完整 tool_use / tool_result 记录，且 `/api/budget` 的真实 spend 增量可见
- 这一步把 cleanup 从“运维/API 管理动作”推进成了“agent 可自助执行的治理动作”，后续若要做 dashboard 按钮或 orchestration hook，就不必再重新设计一套 cleanup 语义。

### 3.12 Lifecycle / Tag Snapshot 进入 Dashboard

- dashboard 的 Memory 页现在不再只是 `key / value / delete` 的简单 KV 浏览器，而是开始直接消费 governed memory 响应中的治理字段。
- 当前 dashboard 已展示：
  - `namespace`
  - `kind`
  - `freshness`
  - `tags`
  - `source`
  - `lifecycle_state`
  - `review_at`
  - `expires_at`
  - `promotion_candidate`
- dashboard Memory 页新增了与治理 API 对齐的过滤器：
  - `namespace`
  - `lifecycle`
  - `tags`
  - `include_internal`
  - 本地 search
- 这样 dashboard 不需要另起一套前端私有过滤语义，而是直接沿用现有 memory API 的治理查询参数。
- 页面顶部还新增了 summary cards，用于快速观察：
  - loaded keys
  - governed count
  - active / stale count
  - promotion candidates
- Add/Edit key 表单也已同步支持可选：
  - `kind`
  - `tags`
  - `freshness`
- 这使 dashboard 从“只能改 value 的 KV 控制台”提升成了“能观察和编辑治理元数据的 Memory 工作台”。
- live verification 已确认这条链路不是静态页面装饰：
  - dashboard HTML 已包含 `Memory Filters` / `Promotion Candidates` / `Active / Stale`
  - 通过 API 写入 `pref.dashboard_lifecycle_probe.theme` 后，单条读取返回了完整 lifecycle + promotion 字段
  - list API 也能通过 namespace / tags / lifecycle 查询命中同一 probe，说明 dashboard 过滤器与后端语义一致
- 这一步仍然没有把 lifecycle / promotion 真正提升为“prompt orchestration 的决策输入”，但它已经让治理状态进入了可操作、可观测的 UI 层，为后续 orchestration 提供了更稳定的人工 inspection 面。

### 3.13 Cleanup Audit / Apply 进入 Dashboard

- dashboard 的 Memory 页现在新增了 `Governance Cleanup` 面板，直接消费现有 `/api/memory/agents/:id/kv/cleanup` 接口，而不是再造一套前端私有 cleanup 语义。
- 当前 dashboard 已支持：
  - 配置 cleanup `limit`
  - 手动触发 audit
  - 手动触发 apply，并在执行前二次确认
  - 展示 audit/apply 模式、最近一次运行时间与 applied summary
  - 展示 findings / migrate legacy / orphan metadata / backfill metadata 四类 summary cards
  - 展示逐条 finding 明细，包括 `action` / `key` / `canonical_key` / `metadata_key`
- apply cleanup 完成后，dashboard 会自动刷新当前 memory 列表，因此 legacy bare key 迁移、orphan sidecar 删除与 metadata 回填的结果可以立即在同一页观察到。
- 这样 cleanup 现在同时具备三层可达入口：
  - API：运维或脚本显式调用
  - tool：agent 自助治理
  - dashboard：人工 inspection 与手动治理
- 这一步的意义不是新增 cleanup 规则，而是把既有 cleanup 规则推进成真正可操作的治理工作台，减少“需要 curl 才能治理”的摩擦。
- live verification 已确认这条 UI 链路真实工作：
  - dashboard HTML 已出现 `Governance Cleanup` / `Audit Cleanup` / `Apply Cleanup`
  - 注入 legacy bare key、missing metadata 与 orphan sidecar 三类 probe 后，cleanup audit 返回了对应 findings；同一轮里也顺带暴露出几条原本就存在的 shared legacy bare key
  - cleanup apply 后，canonical key 与 metadata 回填可被 API 读回，而 orphan sidecar 已删除；验证过程中被一并迁移的无关 shared legacy bare key 已恢复回原状
  - 现有 `Researcher` 与 `assistant` agent 的真实 message 分别命中了既有的 MiniMax tool-result id 错误与 Gemini `thought_signature` 错误，因此最终用一个临时无工具 MiniMax verifier agent 补齐了真实 LLM + `/api/budget` 增量验证

### 3.14 Lifecycle / Promotion 进入 Prompt Orchestration

- governed retrieval 之前已经把 lifecycle / promotion 字段放进 `Governed memory candidates` 明细里，但那仍然要求模型自己从候选行中归纳“哪些 stale 需要复核、哪些 durable 该进 MEMORY.md”。
- 现在 `openfang-types::memory` 新增了 governed orchestration snapshot helper，会围绕当前 query 直接总结两类动作性信号：
  - `stale_review`
  - `promotion_candidates`
- runtime 在构造动态 memory context 时，会先注入新的 `Governance attention signals` 小节，再附加原有 `Governed memory candidates`：
  - `Review stale memory before reuse: [...]`
  - `Consider promoting to MEMORY.md: [...]`
- 这样 lifecycle / promotion 不再只是“字段可见”，而是第一次进入了 prompt orchestration 的上层决策提示，模型不需要再自己从原始 metadata 行里二次推理。
- 当前 signal 仍然复用既有 governed retrieval 语义：
  - query-aware 匹配
  - lifecycle 排序
  - promotion candidate 规则
  - tag / kind / freshness / updated_at
- 但它把这些规则压缩成了更直接的动作提示，适合驱动：
  - stale 记忆先复核再复用
  - durable preference / decision / profile 类记忆考虑晋升到 `MEMORY.md`
- live verification 已确认这不是只在代码里拼字符串：
  - 通过 API 写入两条 shared governed probe，并把 sidecar `updated_at` 调到 stale 窗口
  - verifier agent 的 `llm.log` 中真实出现了 `Governance attention signals`
  - log 中明确包含两条 stale review 和一条 promotion 提示
  - 同轮回复也按 stale review / promotion 结构组织，说明该摘要已经被模型实际消费
- 这一步仍然没有实现“自动 cleanup / 自动 promotion”，但它已经把治理状态从被动可见推进到了主动提示，基本完成了 Phase 1 对 prompt orchestration 的最小闭环。

### 3.15 Strengthened Query Profile for Governed Retrieval

- 在最小 query-aware 排序之上，`openfang-types::memory` 现在新增了一层共享 query profile，专门为 governed retrieval / orchestration 做更可解释的 query 归一化。
- 当前 profile 会做三件事：
  - 过滤常见 stopwords，避免 `what / is / the / right / now` 之类无信息词稀释打分
  - 从剩余有效词里提取 2-gram / 3-gram phrases，给 `project alpha status` / `qa signoff` 这类短语更高权重
  - 从 query terms 归纳 namespace / kind hints，例如：
    - `format` / `style` / `theme` / `prefer` → `pref` / `preference`
    - `requirements` / `policy` / `must` → `constraint`
    - `project` / `status` / `launch` / `blocked` / `qa` → `project` / `project_state`
- 打分时会综合消费：
  - exact tag hit
  - key token / namespace / kind hit
  - normalized key/value phrase hit
  - namespace / kind hint hit
- 这样 governed retrieval 不再只依赖字面 token 命中，而是具备了最低限度的 query intent 归纳能力。
- 这层能力同时被 `select_governed_memory_prompt_candidates_for_query` 和 `summarize_governed_memory_orchestration_for_query` 复用，因此 prompt candidates 和 governance attention signals 共享一套排序语义，而不是各自漂移。
- 新增单测已覆盖：
  - stopword 去除与 phrase 构造
  - preference hint 把 reply-style preference 拉到 project-state 之前
- live verification 也确认该增强真实进入了运行链路：
  - 当用户问 `How should you format replies for me?` 时，`llm.log` 中 `Governed memory candidates` 前三为 `pref.query_profile.reply_style`、`general.query_profile.response_rule`、`project.alpha.query_profile.status`
  - 当用户问 `What is blocking the alpha launch?` 时，同一份 log 中排序切换为 `project.alpha.query_profile.status`、`general.query_profile.response_rule`、`pref.query_profile.reply_style`
  - 第一轮真实回复同时引用了 reply-style preference 与 no-table constraint，第二轮则直接返回 `Alpha launch is blocked on QA signoff.`
- 这一步仍然不是 embedding / hybrid retrieval，也还没有进入显式 filtering / action hook，但已经把 governed retrieval 从“轻量词命中”推进到了“带 query profile 的可解释排序”。

### 3.16 Cleanup Findings 进入 Prompt Orchestration

- cleanup 之前已经具备三种显式入口：
  - API audit/apply
  - `memory_cleanup` tool
  - dashboard cleanup workspace
- 但在这之前，agent 每轮请求并不会自动知道 shared memory 池里是否存在治理异常；它只有在主动调用 `memory_cleanup` 或人工 inspection 时才看得见这些问题。
- 现在 `openfang-types::memory` 新增了一层 cleanup orchestration snapshot，把 `plan_memory_cleanup()` 的 findings 按三类动作分桶：
  - `legacy_repairs`
  - `metadata_repairs`
  - `orphan_metadata`
- runtime 在构造动态 memory context 时，会先把这些 finding 压缩成新的 `Governance maintenance signals` 小节，再继续注入既有的：
  - `Governance attention signals`
  - `Governed memory candidates`
- 当前 maintenance signals 会直接提示模型：
  - `Run memory_cleanup before reuse: migrate legacy key [...] to [...]`
  - `Run memory_cleanup to backfill governed metadata for [...]`
  - `Run memory_cleanup to remove orphan metadata sidecar [...]`
- 这一步的意义在于：cleanup 首次从“工具/API 可达能力”前移成“prompt-time 可见的治理异常”，模型不需要先自己怀疑 memory 池是否脏了。
- shared helper 仍然保持单点语义：
  - cleanup 规则继续由 `plan_memory_cleanup()` 决定
  - prompt orchestration 只是消费它的 snapshot，不再另写一套运行时推断逻辑
- live verification 已确认这不是只在单测里生效：
  - 在 daemon 停止状态下，直接向 shared `kv_store` 注入了三类异常：legacy bare key、缺失 metadata 的 canonical key、orphan metadata sidecar
  - cleanup audit endpoint 返回了对应 findings，同时也暴露出几条原本就存在的 shared legacy bare key
  - verifier agent 的 `llm.log` 中真实出现了 `Governance maintenance signals`
  - log 中明确包含 `project.maintenance_signal.note` 的 metadata backfill 提示和 `__openfang_memory_meta.pref.maintenance_signal.orphan` 的 orphan sidecar 删除提示
  - 由于 legacy bucket 当前限制为 2，prompt-time signal 优先展示的是两条既有 shared legacy bare key；本次注入的 `maintenance_signal_legacy_probe` 通过 cleanup audit 被确认命中
  - verifier 的真实回复明确说明 shared memory 在复用前需要治理，并指出当前没有 `memory_cleanup` tool 可直接执行
- 这一步仍然没有实现“自动 cleanup”，但它把 maintenance 从被动工具能力推进成了主动治理提示，和 lifecycle / promotion attention 一起构成了 Phase 1 的最小 orchestration 闭环。

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
- `crates/openfang-runtime/src/agent_loop.rs`
  - 动态 memory context 现在会加载 governed KV 候选
  - governed 候选与 session summary 共用同一份 structured KV snapshot，避免 runtime 再各自定义一套筛选逻辑
- `crates/openfang-runtime/src/prompt_builder.rs`
  - prompt 协议补充 namespaced key 约束，以及 tags/lifecycle 使用提示
  - 动态 memory context 新增 `Governed memory candidates` 区段
- `crates/openfang-kernel/src/wizard.rs`
  - setup hint 补充 namespaced key、tags 与 lifecycle 指导
- `crates/openfang-api/src/routes.rs`
  - memory API 对齐治理规则
  - API 列表过滤、governed metadata 折叠与 lifecycle 返回
  - API 列表 tags 过滤
  - cleanup audit/apply endpoint

## 6. 下一步建议

在当前切口稳定后，下一步按以下顺序推进：

1. 评估 maintenance signals 在 agent 具备 `memory_cleanup` 能力时，是否应该进入更主动的 orchestration hook，而不只停留在当前 prompt 摘要。
2. 评估 governance attention signals 是否需要进一步驱动自动 promotion / cleanup，而不只停留在当前 prompt 摘要层。
3. 评估 governed retrieval 是否还需要更强的显式过滤或 action hook，而不只是当前 query profile + 排序增强。
