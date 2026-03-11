# Tool 渐进式发现与展开方案

## 概览

本文档最初提出一套面向 OpenFang 模型可见能力的渐进式发现架构。

经过 Phase 1 落地与后续设计复盘，本文档现在同时承担两件事：

- 记录已经完成的 Phase 1 `skill_search -> skill_get_instructions` 方案
- 记录下一阶段向 Anthropic `tool_search` / `defer_loading` 模型收敛时，已经确认的设计约束

长期目标形态是：

- 默认 system prompt 不再包含完整的 bundled skills、installed skills 或其它可展开能力源目录
- 模型按需发现相关 tool
- runtime 支持清晰的 `discover -> auto-expand -> tool_use` 链路
- 架构采用渐进式迭代，而不是一次性统一所有能力来源

这份文档不再把问题限定为“skill progressive loading”，而是升级为更通用的模型：

- `skill` 是一种 `tool resource`
- `mcp tool` 是一种 `tool resource`
- `builtin tool` 是一种 `tool resource`

这也包括当前已经存在的 agent 协调类 builtin tools：

- `agent_send`
- `agent_spawn`
- `agent_list`
- `agent_kill`

但有一个边界必须明确：

- `capability` **不是** `tool resource`
- `capability` 继续作为授权边界，负责控制可见性和可执行性

## 当前现状

OpenFang 在执行层已经做了部分统一，但在发现、授权、延迟加载三件事上仍然没有统一抽象。

当前代码中可以确认的事实：

- 模型最终看到的是统一的 `Vec<ToolDefinition>`
- `ToolDefinition` 当前只有三个核心字段：
  - `name`
  - `description`
  - `input_schema`
- `kernel.authorized_tools()` 当前会把三类能力合并成一个 tool 列表：
  - builtin tools
  - skill-provided executable tools
  - MCP tools
- `tool_runner.rs` 当前的执行路径也是统一的：
  - builtin tools 通过 match arm 直接执行
  - MCP tools 通过 `mcp_` 命名空间 fallback 执行
  - skill-provided tools 通过 skill registry fallback 执行

所以，**LLM tool protocol 已经统一**：

- kernel 先组装 `authorized_tools`
- agent loop 通过有状态 `ToolRunner` 维护当前轮 `visible_tools`
- 每轮 LLM 请求只把 `visible_tools.to_vec()` 放入 `CompletionRequest.tools`
- driver 将这些 `ToolDefinition` 原样传给 provider 的 tool API
- provider 返回结构化 `tool_use`
- runtime 再统一解析成 `ToolCall`

但下面这些仍然没有统一：

- discovery
- deferred loading
- automatic expansion
- 权限语义
- 来源建模

当前存在的割裂点包括：

- 当前对外 discovery 已经收敛到 `tool_search`
- `authorized_tools` 已经有了清晰语义，`ToolRunner` 也已经按 `defer_loading` 维护 loop-local `visible_tools`
- skill 和 MCP 虽然最终都变成 `ToolDefinition`，但授权规则并不统一
- builtin 更依赖 `Capability::ToolInvoke`
- skill 主要受 `skill_allowlist + model_invocable + host_tools`
- MCP 主要受 `mcp_servers` allowlist
- 因此不能假设“全体工具先统一授权，再统一搜索”的现成前提已经存在

## 问题定义

当前系统有三个结构性问题：

1. 执行面已经统一，但发现面仍然是碎片化的
2. prompt 里还保留着大型 skill 目录，而 runtime 执行层其实已经是 tool 化的
3. 新能力源仍在以 special case 的方式接入，而不是走统一 discovery protocol

这会带来四个具体问题：

1. 静态目录导致 token 浪费
2. prompt cache 效率下降
3. 无关 skill summary 稀释模型注意力
4. skills、MCP 以及未来能力源无法共享一套稳定的装配协议

## 设计目标

- 建立统一的模型侧 discovery protocol，用于发现可展开的 tool resources
- 保留当前已经存在的统一执行面
- 渐进式 rollout，从 Phase 1 skills-only 过渡到统一 tool discovery
- v1 保持简单、可解释
- 避免过早引入 embeddings 或过度复杂的 orchestration
- 保留现有 capability / permission 模型

## 非目标

- 在 v1 里彻底重写 builtin tool runtime
- 一次性把所有能力源都改成 deferred loading
- 把 capabilities 当成 tools
- 在 v1 引入向量检索
- 在 v1 构建通用语义检索平台

## 核心架构决策

OpenFang 应把所有“**可被模型发现和使用的能力单元**”统一抽象为 `Tool Resource`，同时把 `Capability` 保留为独立的授权层。

这意味着：

- skill manual 是 `tool resource`
- skill-provided executable commands 是 `tool resource`
- MCP tools 是 `tool resource`
- builtin tools 是 `tool resource`

但下面这些仍然是授权边界，而不是 tool：

- `Capability::ToolInvoke`
- `Capability::ToolAll`
- agent 的 skill allowlist
- agent 的 MCP server allowlist

它们继续作为 discover / expand / execute 的硬上界。

## 为什么 Capability 不能被当成 Tool

如果把 `Capability` 也建模成 tool，会把两类问题混在一起：

- “这个能力源是什么”
- “这个 agent 是否被允许看见和使用它”

这样会让授权、审计、排序都变得更难推理。

正确的拆分应该是：

- `Tool Resource`：模型可以发现、可能使用的能力单元
- `Capability`：决定它是否可见、可展开、可执行的边界

## 已确认的设计收敛

经过实现与复盘，已经确认下面几条：

1. 下一阶段应该继续向 Anthropic `tool_search` 方案收敛，而不是继续扩展 `skill_search` 兼容层
2. `tool_reference` 不应该在 OpenFang 内被设计成胖对象；Anthropic-compatible 的外部协议应保持薄
3. 真正需要统一的是 OpenFang 内部的 tool catalog，以及 `tool_search` 命中后的 automatic expansion
4. 当前 runtime 已经支持 `tool_search` 触发 skill 与 MCP deferred tool 的自动展开；builtin tools 仍保持默认 visible，不参与这轮 deferred rollout
5. 下一阶段的关键改动点不是“再设计新的 skill 协议”，而是“把有状态 ToolRunner 的动态工具集从 skills 推广到更通用的 deferred sources”

## 建议的数据模型

### 当前最小内部目标

下一阶段不需要先引入复杂的 `ToolCatalogEntry + metadata` 体系。

更小、更贴近当前代码现实的内部目标是：

- agent loop 入口参数语义收敛为 `authorized_tools`
- runtime 通过有状态 `ToolRunner` 维护当前轮 `visible_tools`
- 后续在 `ToolRunner` 内继续引入 `defer_loading` / hidden tool 集合
- `tool_search` 最终只搜索“当前 agent 已授权且默认不 visible”的工具
- 命中后由 `ToolRunner` 自动将完整 `ToolDefinition` 并入下一轮 LLM 请求
- `skill_search` 则继续作为 skill-scoped compatibility path 保留旧语义

也就是说，最小抽象先收敛为：

```rust
pub struct ToolRunner {
    pub authorized_tools: Vec<ToolDefinition>,
    pub visible_tools: Vec<ToolDefinition>,
}
```

其中：

- `authorized_tools` 是 kernel 已完成授权与 allowlist 过滤后的全集
- `visible_tools` 是当前轮真正发给 LLM 的工具子集

这个阶段不强制引入 `ToolSource`、`metadata` 或其它额外对象。

### 历史目标模型（保留作参考）

runtime 内部可以逐步演进到一个更轻量的 discovery descriptor：

```rust
pub enum ToolSource {
    Builtin,
    Skill,
    Mcp,
    Workflow,
    Agent,
}

pub enum ToolKind {
    Callable,
    Instructional,
    Composite,
}

pub struct ToolReference {
    pub id: String,
    pub source: ToolSource,
    pub kind: ToolKind,
    pub name: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub required_capabilities: Vec<String>,
    pub expandable: bool,
}
```

这只是更远期的参考架构，不是下一阶段的直接实现目标。

### 按来源映射

| 来源 | 类型 | 例子 |
|---|---|---|
| builtin tool | callable | `web_search`, `file_read` |
| skill manual | instructional | `rust-expert` |
| skill command | callable | `summarize_url` |
| MCP tool | callable | `mcp_github_create_issue` |
| workflow | composite | 未来的可复用任务包 |
| agent handoff | composite | 未来可被委派的 agent resource |

### 来源分类 与 语义角色

这里要明确区分两种分类方式：

- **来源分类**：这个能力单元来自哪里
- **语义角色**：它在模型工作流里扮演什么角色

例如：

- `agent_send` 当前按来源属于 **builtin tool**
- 但它按语义属于 **coordination / delegation execution**

这一区分很重要，因为系统不应该混淆：

- 执行 delegation 的底层 builtin tool
- 将来可能被模型选中的高层 delegable agent resource

换句话说：

- 今天：`agent_send` 是 builtin callable tool
- 将来：某个可委派的 agent 可以被建模为 discoverable `agent` resource
- 真正执行时，仍然可能落到底层 builtin `agent_send`

因此 rollout 里不应该把“agent delegation”在第一阶段当成一个全新的 source；
应该承认：delegation 相关 builtin 已经存在，而高层 delegation discovery 模型是后续阶段再处理的东西。

## 目标运行时协议

与 Anthropic `tool_search` 对齐的目标 flow 应该是：

1. 模型判断当前任务可能受益于某种 specialized tool resource
2. 模型调用 discovery tool
3. runtime 在当前 agent 已授权的 deferred tools 中搜索
4. runtime 返回命中的 `tool_reference`
5. `ToolRunner` 自动展开这些 `tool_reference` 对应的完整 `ToolDefinition`
6. 下一轮 LLM 请求携带扩展后的 tools 集合
7. 模型在完整 `ToolDefinition` 中选择并发起 `tool_use`
8. runtime 解析为 `ToolCall` 并执行

也就是说，目标主链路是：

`llm -> tool_search -> host automatic expansion -> llm tool_use -> ToolCall`

这里要特别强调：

- 如果采用 Anthropic-compatible 主链路，`tool_reference` 对外不需要承担“完整描述 + 显式 load”的职责
- OpenFang 的真正设计难点在于 automatic expansion，而不是给 `tool_reference` 发明更多字段

### 当前实现落点（进行中）

为了避免把 hidden/deferred tool 细节散落到 `agent_loop`，当前代码实现先落了一步运行时重构：

- kernel 侧命名从 `available_tools` 收敛为 `authorized_tools`
- `agent_loop` 不再直接持有和重复派生当前轮工具集
- runtime 引入有状态 `ToolRunner`
- 当前每轮 `CompletionRequest.tools` 都从 `ToolRunner.visible_tools()` 读取
- `ToolDefinition` 现在已有正式 `defer_loading` 字段
- 当前实现已经把 **skill-provided callable tools** 以 `defer_loading = true` 的形式从初始 visible 集中拆出
- 当前实现也已经把 **MCP tools** 以 `defer_loading = true` 的形式从初始 visible 集中拆出
- `ToolRunner` 在执行现有 `skill_search` 时，会自动把命中 skill 对应的 callable tools 并入后续轮次的 visible 集
- `ToolRunner` 在执行 `tool_search` 时，会自动把命中的 deferred tools 并入后续轮次的 visible 集
- 这意味着当前已经跑通了一个过渡版链路：
  - `llm -> skill_search -> host automatic expansion of matching skill tools -> llm tool_use`
- 并且已经跑通了第一条真实非-skill 链路：
  - `llm -> tool_search -> host automatic expansion of matching MCP tools -> llm tool_use`

这一步的目标是先把“谁管理当前轮工具面”这个职责从 `agent_loop` 收口到单一对象，并用现有 `skill_search` 验证 automatic expansion 的 runtime 机制。

### 当前 live validation 已确认的行为

- 在真实 MiniMax provider 上，单条顶层 message 内已经验证通过：
  - `skill_search`
  - host automatic expansion of matching hidden skill tools
  - 同一条顶层 message 内继续 `tool_use` 新暴露出的 skill tool
- 在真实 MiniMax provider 上，公开 alias 也已经验证通过：
  - `tool_search`
  - host automatic expansion of matching hidden skill tools
  - 同一条顶层 message 内继续 `tool_use` 新暴露出的 skill tool
- 在真实 MiniMax provider 上，generic deferred MCP flow 也已经验证通过：
  - `tool_search`
  - host automatic expansion of matching hidden MCP tools
  - 同一条顶层 message 内继续 `tool_use` 新暴露出的 `mcp_minimax_web_search`
  - `/api/budget/agents/{id}` 已记录 `live-mcp-tool-searcher` 的真实 spend
- 在重新构建 CLI binary 后，session log 已确认：
  - `tool_search` 当前返回的是 **tool-centric** 结果，而不是 legacy skill result
  - 当前 live 结果形态是：
    - `name`
    - `description`
    - `source`
    - `provider`
- 当前这次 `tool_search` live probe 命中的 `github_comment` 是 prompt-only skill tool：
  - runtime 确实完成了 discovery + expansion + invocation
  - 但工具执行结果按设计只返回 prompt-context note，不会伪造 GitHub side effect
- 外部 `/mcp` transport 当前不会再暴露 loop-only discovery helpers：
  - `tool_search`
  - `skill_get_instructions`
  - 这些能力依赖 loop-local `ToolRunner` state，因此应保留在 agent loop 内部而不是对 stateless MCP client 公开
- 标准 `ToolProfile` 与 prompt 主文案现在都已切到 `tool_search`：
  - `skill_search` 已经不再存在于默认 surface，也不再保留 builtin alias
- `skill_get_instructions` 目前仍保留在默认 surface：
  - 主要原因是 prompt-only skill 仍可能需要按需加载详细规则
  - 因此它还没有进入删除阶段
  - 当前 `tool_search` 已经开始返回 `skill_manual` 类型结果，模型可以先统一 discovery，再按需调用 `skill_get_instructions`
- 这个 expanded tool surface 当前是 **loop-scoped** 的，而不是 session-scoped：
  - 同一 agent 的下一条新顶层 message 会重新从初始 visible tool set 开始
  - 如果要再次使用 deferred skill tool，需要在新的顶层 message 中重新经过 discovery / expansion
- 现有 legacy `assistant` agent manifest 的验证应以 `tool_search` 为准
  - 所以 live 验证应使用 fresh probe agent 验证 `tool_search -> auto-expand -> tool_use`

### `skill_search` 什么时候废除

`skill_search` 已在本阶段移除。

实际迁移顺序已经落成：

1. 先把内部语义统一到显式 `defer_loading` + loop-local hidden tool 集
2. 现在已经引入公开的 `tool_search` 名称，并在 prompt、tool profiles 里把 discovery 主语义切向它
3. 当前这版 `tool_search` 已经覆盖 skill 与 MCP deferred tools；builtin tools 继续保持 `defer_loading = false`
4. 标准 profile / prompt 主语义已经全部切到 `tool_search`
5. runtime、kernel、builtin catalog 中的 `skill_search` 兼容入口也已经删除

也就是说：

- **`tool_search` 已经成为唯一 discovery 入口**
- **`skill_search` 不再对内或对外暴露**
- **`skill_get_instructions` 仍保留，但现在已经是 `tool_search` 统一 discovery 后的二段式 manual loader**

## 渐进式 rollout 策略

这套架构应该渐进式采用。

### Phase 1：只做 Skills

这是已经完成的第一阶段实现范围。

覆盖：

- bundled skills
- user-installed skills
- workspace skills

行为：

- 保留 skill-provided executable tools 的现有执行路径
- 去掉默认 prompt 中的全量 skill catalog
- 为 installed skills 增加 runtime discovery
- 保留按需加载详细 skill instructions 的方式

Phase 1 接口：

- `skill_search(query, top_k)`
- `skill_get_instructions(skill_name)`

Phase 1 的关键约束是：

**Phase 1 改的是结构，不是外部命名。**

不要急着把所有接口都改成 `tool_*`，原因很简单：

- 系统里已经有 `skill_get_instructions`
- 第一阶段真正新增的价值是 discovery，不是 rename
- 延后重命名可以显著降低迁移风险

### Phase 2：接入 MCP

skill 流程稳定之后，再把同一套抽象应用到 MCP tools。

覆盖：

- 当前已经缓存到 `kernel.mcp_tools` 里的 connected MCP tools

行为：

- 不再把 MCP 只当成“prompt summary + always-visible callable surface”
- 增加 MCP resource 的 discovery
- 支持 reference selection 和 callable schema expansion

这个阶段会验证这套抽象是否真的能同时容纳：

- instructional resources（skill）
- callable resources（MCP）

### Phase 3：其它能力源

只有在 skill 和 MCP 都稳定后，才应该把同样抽象扩展到：

- builtin tools
- workflows
- agent delegation targets
- 未来的其它 composite resources

这样可以避免 v1 演变成整套 capability system 的重写。

补充说明：

- 低层 agent coordination tools 今天已经是 **builtin tools**
- Phase 3 要增加的不是“这些 tools 是否存在”
- 而是更高层的 delegable agent resource discovery model

也就是说：

- `agent_send` / `agent_spawn` 继续归类为 builtin tools
- 未来“delegate to researcher”这一类高层资源，再单独建模在它们之上

## Phase 1 设计：Skill Discovery

### 当前 Skill 行为

今天 OpenFang 已支持：

- installed skill registry
- 可见 skill 的 prompt catalog injection
- `skill_get_instructions(skill_name)`
- 通过 registry dispatch 执行 skill executable tools

缺的只有一层：

- 本地运行时 installed skills 的自然语言 discovery

### Phase 1 目标

在下面两步之间补一层 discovery：

- “skills exist”
- “load one skill manual”

### 目标模型工作流

1. 判断当前任务是否可能受益于 specialized guidance
2. 调用 `skill_search("natural language intent", top_k=3)`
3. 查看返回的 short list
4. 选出最强候选，或者决定跳过
5. 调用 `skill_get_instructions(skill_name)`
6. 执行任务

### Phase 1 的 Prompt Strategy

把默认 prompt 里的全量 skill catalog 替换成简短 protocol：

```md
## Skills
- Skills are available on demand.
- Do not assume a skill is relevant just because it exists.
- When a request may benefit from specialized guidance, search for matching skills first.
- If a skill looks relevant, load detailed instructions only for that skill.
```

这样既保留了技能使用协议，又不再让所有 skill summary 常驻占用 prompt。

## 检索策略

### 原则

不要要求模型去生成精确关键词。

更稳的方式是：

- 让模型直接发送自然语言任务意图
- 由后端负责 matching 和 ranking

### V1 检索方法

Phase 1 应采用轻量 lexical scorer，对 installed skill registry 做匹配。

候选信号包括：

- exact skill name match
- prefix match
- alias match
- description token hits
- tag hits
- provided tool hits
- 是否存在 prompt_context 的小幅加分

### Query Normalization

进入打分前先做：

- lowercase
- tokenize
- drop common stopwords
- 扩展一小批 alias / synonym table

例如：

- `k8s -> kubernetes`
- `review -> code-reviewer`
- `email -> email-writer`
- `ts -> typescript`
- `rag -> retrieval, vector, embeddings`

### Ranking

v1 排序应保持简单、可解释。

示例：

```text
score =
  10 * exact_name_match +
   6 * prefix_name_match +
   4 * alias_match +
   3 * description_hits +
   2 * tag_hits +
   1 * provided_tool_hits +
 0.5 * has_prompt_context
```

具体系数可以调整，但“可解释性”比“复杂度”更重要。

## Reference 与 Expansion 语义

Phase 1 不必立刻实现 Claude 那种自动 reference expansion，但架构上应预留这条路径。

建议演进方式：

### Phase 1a

- `skill_search` 返回 compact results
- 模型再调用 `skill_get_instructions(skill_name)`

### Phase 1b

内部引入一个更稳定的 reference 概念，例如：

```json
{
  "name": "rust-expert",
  "kind": "instructional",
  "source": "skill",
  "score": 0.91,
  "match_reason": ["tag:rust", "description:ownership"]
}
```

### Phase 2+

再把它进一步泛化成真正的 `tool_reference` 风格，适配 skill 和 MCP 两类资源。

## 吸收 Claude 经验，但不过度照搬

Claude 的 tool search 设计有一个很强的点：它清晰分离了：

- 少量 always-available search surface
- 大量 deferred resources
- 明确的 discover 之后再 expand

OpenFang 最值得借鉴的是：

- 把 always-visible surface 保持得尽量小
- 把 search 视为 runtime protocol，而不是一个便捷小功能
- 优先用 references 和 lazy expansion 代替静态目录
- 少量高频资源常驻，长尾资源延迟加载

但不应该过早照搬的是：

- 在 skill discovery 还没稳定时，就引入完整自动 expansion 协议
- 在第一阶段就对所有能力源做全 deferred rewrite

## 现有代码触点

### 当前 Prompt 层

- [crates/openfang-runtime/src/prompt_builder.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-runtime/src/prompt_builder.rs)

当前行为：

- 每轮注入完整 `## Skills` summary
- 单独注入 MCP summary
- callable tools 则通过正常 tool list 暴露

Phase 1 改动：

- 移除全量 skill enumeration
- 换成简短的 skill protocol

### 当前 Runtime 执行层

- [crates/openfang-runtime/src/tool_runner.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-runtime/src/tool_runner.rs)

当前行为：

- builtins 直接执行
- `skill_get_instructions` 加载单个 skill manual
- `mcp_` names 通过 MCP fallback 执行
- skill executable tools 通过 skill registry fallback 执行

这里的 builtin tools 既包括直接执行型工具，也包括协调型工具，例如：

- 直接执行：`file_read`、`web_search`、`shell_exec`
- 协调执行：`agent_send`、`agent_spawn`、`agent_list`、`agent_kill`

Phase 1 改动：

- 新增 `skill_search`

Phase 2 改动：

- 为 MCP 增加 discovery / expansion 支持

### 当前 Skill Registry

- [crates/openfang-skills/src/registry.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-skills/src/registry.rs)

当前行为：

- load bundled / user / workspace skills
- list skills
- get by name
- gather executable tool definitions
- find tool provider

Phase 1 改动：

- 为 installed skills 增加本地 runtime search
- 输出 compact ranked results

### 当前 Kernel 聚合层

- [crates/openfang-kernel/src/kernel.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-kernel/src/kernel.rs)

当前行为：

- `available_tools()` 合并 builtin + skill executable tools + MCP tools
- `collect_skill_info()` 生成 prompt-time skill summaries
- `build_mcp_summary()` 生成 prompt-time MCP summaries

Phase 1 改动：

- 停止用 `collect_skill_info()` 做全量目录注入

Phase 2 改动：

- 让 MCP 从“仅 summary 暴露”演进到 discovery-capable resource exposure

## API Surface 建议

### Phase 1

增加一个 runtime 内置工具：

```json
{
  "query": "natural language task intent",
  "top_k": 3
}
```

建议返回：

```json
{
  "results": [
    {
      "name": "rust-expert",
      "description": "Rust programming expert for ownership, lifetimes, async/await, traits, and unsafe code",
      "tags": ["rust", "systems"],
      "tools_count": 0,
      "has_prompt_context": true,
      "score": 0.91,
      "match_reason": ["tag:rust", "description:ownership"]
    }
  ]
}
```

后续可选 API 复用：

- `POST /api/skills/search`

但建议它跟在 runtime tool 之后实现，而不是先做 HTTP API。

## 错误与弱结果策略

v1 文档里应该把这些行为写死。

### No Results

如果 `skill_search` 没有找到强候选：

- 返回空的 `results`
- 模型正常继续，不加载 skill

### Weak Results

如果结果都比较弱：

- 仍然返回 top results，但带较低分数
- 模型应被鼓励跳过 skill loading

### Missing Skill on Expand

如果 `skill_get_instructions(skill_name)` 被用于：

- unknown skill
- disabled skill
- 当前不可见的 skill

则应明确报错，而不是 silently degrade

## 验收标准

### Phase 1

- 默认 system prompt 不再列出全部 skills
- 模型可以通过 `skill_search` 发现 installed skills
- 模型只为被选中的 skill 加载详细 instructions
- 常规 turn 的 prompt size 有明显下降
- skill selection quality 比“直接猜 skill name”更好

### Phase 2

- MCP tools 能参与同一套 discovery / expansion 模型
- MCP prompt exposure 不再依赖大段静态 summary
- 模型可以发现 callable external tools，而不是一开始就拿到全量暴露

## 总结

推荐方向是：

1. 承认 OpenFang 在执行层已经有统一的 `ToolDefinition` surface
2. 在此基础上补统一的 discovery architecture
3. 保留 `Capability` 作为独立授权层
4. 采用渐进式 rollout：
   - 先 skills
   - 再 MCP
   - 最后其它能力源
5. 用 `skill_search -> skill_get_instructions` 作为第一版、也是最小可行的 `discover -> reference -> expand` 实现

这是当前最小、最稳的路径，它能同时做到：

- 立即改善 prompt 质量
- 与 Claude 的 deferred discovery 思路对齐
- 避免整套系统一次性重构
- 为后续真正统一的 tool resource 架构打基础
