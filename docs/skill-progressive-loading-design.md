# Tool / Skill 渐进式发现方案

## 概览

本文档只描述这个分支**当前已经落地的实现**，以及已经确认的技术决策依据。

这条分支的目标已经不再是早期的 `skill_search -> skill_get_instructions` 方案，而是更通用的：

- 统一使用 `tool_search`
- 统一使用 `tool_get_instructions`
- 统一用 `ToolDefinition.defer_loading` 控制默认可见性
- 由 runtime 在 agent loop 内自动展开 deferred tools

当前分支里，`tool` 是统一抽象；`builtin`、`skill`、`MCP` 只是来源。

## 当前实现结论

### 1. LLM 看到的工具协议已经统一

当前代码里，LLM 最终拿到的是统一的 `Vec<ToolDefinition>`：

- builtin tools
- skill-provided callable tools
- MCP tools

运行链路是：

1. kernel 先计算 agent 的 `authorized_tools`
2. runtime 的 `ToolRunner` 再维护 loop-local `visible_tools`
3. 每轮只把 `visible_tools` 发给 LLM
4. 模型返回 `tool_use`
5. runtime 统一解析成 `ToolCall`
6. 再按现有执行分发去执行 builtin / skill / MCP

这里已经成立的统一抽象是：

- `ToolDefinition`
- `ToolCall`
- `ToolResult`

### 2. `tool_search` 已经是唯一 discovery 入口

当前分支中：

- `skill_search` 已删除
- `tool_search` 是唯一 discovery 入口
- `tool_get_instructions` 是统一的 guidance/manual loader

它们都已经是 runtime builtin tools。

### 3. agent loop 已支持 automatic expansion

当前实现不是“搜索后只返回名字，模型再显式 load”，而是：

1. 模型调用 `tool_search`
2. runtime 在当前 agent 已授权但默认隐藏的资源里搜索
3. `ToolRunner` 自动把命中的 deferred tools 并入当前 loop 的 visible tool set
4. 同一条顶层消息内，模型可以继续直接调用新暴露出的 tool

这是当前分支最关键的运行时能力。

### 4. 当前默认可见性策略

这条分支里，当前默认行为是：

- builtin tools：默认 `defer_loading = false`
- MCP tools：默认 `defer_loading = false`
- skill：系统默认 `defer_loading = false`
- bundled skills：当前已统一显式标记为 `defer_loading: true`

因此今天的真实效果是：

- builtin 默认直接可见
- MCP 默认直接可见
- 用户安装 / 工作区 skill 默认直接可见
- 系统内置 bundled skills 默认 deferred，通过 `tool_search` 发现

这条策略是当前分支的**真实代码状态**，不是未来设想。

## 当前 skill 侧配置模型

### 已实现的配置点

skill 侧当前已经支持在 skill 顶层声明 `defer_loading`。

#### `SKILL.md` 顶层 frontmatter

```yaml
---
name: github
description: GitHub operations expert
defer_loading: true
---
```

#### `skill.toml`

```toml
[skill]
name = "github"
description = "GitHub operations expert"
defer_loading = true
```

两种格式最终都会收敛到 canonical `SkillManifest.skill.defer_loading`。

### 当前 skill 默认值

如果 skill 没有显式声明 `defer_loading`：

- 默认值是 `false`

也就是说，系统默认不会把所有 skill 都隐藏。

这是这条分支里明确确认过的技术决策。

### 当前 bundled skill 策略

为了避免内置 skill 目录过大、过噪、对 prompt 造成干扰，这个分支已经把 `crates/openfang-skills/bundled/**/SKILL.md` 全部显式加上：

```yaml
defer_loading: true
```

所以 bundled skills 现在的行为不是依赖代码硬编码，而是来自 skill 自身 manifest/frontmatter。

## 当前 skill 策略的决策依据

### 为什么 skill 默认值不是 `true`

这条分支最终没有采用“所有 skill 默认 deferred”的策略，原因很明确：

1. 这会让用户完全失去控制权
2. skill 是 OpenFang 最主要的扩展方式之一
3. 用户通常希望 agent 先看到用户自己安装的能力包
4. 把全部 skill 默认隐藏，会让能力地图严重退化

所以最终收敛为：

- skill 系统默认 `false`
- 是否 deferred 由 skill 自己声明

### 为什么不做 agent 级 skill override

这一点在这个分支上已经明确排除了。

没有采用“agent manifest 再覆盖 skill 的 defer_loading”，原因是：

1. 对 skill 来说，安装本身已经有作用域：
   - 全局安装
   - 工作区安装
2. 再引入 agent 级 override，会把作用域、授权、可见性混在一起
3. 当前产品模型里，skill 更像一个能力包，不值得为了 deferred 再加一层 agent policy

因此当前 skill 方案刻意保持简单：

- skill 自身 manifest/frontmatter
- 系统默认值

就这两层。

### 为什么 bundled skill 要统一 deferred

这是当前分支里非常明确的一条决策：

- bundled skill 数量多
- 覆盖面广
- 如果默认全量展开，会重新把 prompt 变回超长 catalog

而 bundled skills 的设计目标更适合“长尾能力库”：

- 默认隐藏
- 需要时通过 `tool_search` 发现
- 命中后再按需 `tool_get_instructions`

因此 bundled skills 统一 `defer_loading: true`，是为了降低默认 prompt 噪音，同时保留完整 discoverability。

## 当前 MCP 侧状态

### 已实现行为

当前分支里，MCP tools 已经纳入统一 `ToolDefinition` 和 `ToolRunner` 体系。

但**MCP 的 `defer_loading` 配置项还没有实现**。

当前真实行为是：

- MCP tools 默认 `defer_loading = false`
- 也就是默认直接可见

### 尚未实现的部分

我们已经确认过更合理的 MCP 配置方向，但这条分支**还没有落地**：

- server 级默认值
- tool 级覆盖
- 命名建议对齐 Anthropic 的 `default_config.defer_loading`

因此文档必须明确：

- skill 的 deferred 配置已经实现
- MCP 的 deferred 配置**尚未实现**

不要把这两者写成已经对称完成。

## 当前 prompt 结构

当前 system prompt 已按这个分支的真实实现调整为：

1. `## Tool Use Strategy`
2. `## Immediate Tools`
3. `## Skills`
4. `## Tool Discovery`
5. `## Connected Tool Servers (MCP)`

### 各 section 的职责

#### `## Tool Use Strategy`

描述模型如何决策：

- 当前 visible tools 是否已覆盖任务
- 是否应先走 discovery
- skill manual 什么时候值得先读

它不再使用旧的“只要需要就立刻调工具”式表述，而是和 deferred loading 协调。

#### `## Immediate Tools`

这是“当前已经可直接调用”的统一 tool surface。

这里不是 builtin-only，而是统一包含：

- visible builtin tools
- visible skill tools
- visible MCP tools
- discovery tools

也就是说，它按“当前能不能直接调用”组织，而不是按来源组织。

#### `## Skills`

这里承担 capability map 的职责。

它列出当前 agent 环境下**默认可见的 skill 摘要**，并保留：

- `[manual available]`
- `tool_get_instructions(<skill name>)` 的 skill-specific 指引

当前实现里，这个 section 会受 skill 顶层 `defer_loading` 影响：

- `defer_loading = false` 的 skill 可以进入 `## Skills`
- `defer_loading = true` 的 skill 不进入这个 section

#### `## Tool Discovery`

这是统一 discovery protocol：

- 当前 visible tools 不够时调用 `tool_search`
- 命中 guidance 时调用 `tool_get_instructions(<result name>)`
- deferred tool 被展开后，按精确 tool name 调用

这个 section 不再挂在 `## Skills` 下面，而是独立存在。

#### `## Connected Tool Servers (MCP)`

这里只承担 MCP 环境说明。

当前仍保留明确约束：

```text
To use these tools, call them by their FULL name exactly as shown above.
```

这是因为 MCP namespaced tool name 对模型调用正确性仍然重要。

## 当前运行时行为

### 已验证通过的真实链路

这条分支已经做过真实 MiniMax 联调验证，确认以下行为成立：

#### skill / MCP deferred tool automatic expansion

```text
llm -> tool_search -> host automatic expansion -> llm tool_use
```

#### prompt-only skill guidance flow

```text
llm -> tool_search -> tool_get_instructions
```

#### budget / metering side effect

真实 LLM 调用后，`/api/budget` 与 `/api/budget/agents/{id}` 已确认会更新。

### 当前 deferred expansion 的作用域

当前 expanded tool surface 是：

- loop-scoped

不是：

- session-scoped

这意味着：

- 同一条顶层消息里，discovery 后展开的 tool 可以继续直接调用
- 新的一条顶层消息开始时，会重新从初始 visible tool set 开始

## 与 Anthropic 设计的关系

### 当前已经吸收的部分

这条分支已经采纳了 Anthropic `tool_search` 设计里的几个核心思想：

1. 用统一 discovery tool，而不是 skill-only discovery
2. 用 `defer_loading` 控制默认可见性
3. 把 discovery 后的 tool 自动展开，而不是要求模型手写复杂 load 流程
4. 把 prompt 从“静态大目录”改成“即时工具面 + 能力地图 + discovery protocol”

### 当前没有照搬的部分

有些 Anthropic 设计点在这条分支还没有照搬：

1. 外部协议层的薄 `tool_reference`
2. MCP 的 `default_config.defer_loading` / per-tool override 配置
3. 全部 deferred resource 的统一外部 schema

这里必须明确：

- 这条分支是“向 Anthropic 收敛”
- 不是“已经完整等价于 Anthropic”

## 当前未完成项

下面这些在当前分支里还没有实现：

### 1. MCP 的配置化 deferred policy

还没有：

- `config.toml` 里的 server 级 `defer_loading`
- `config.toml` 里的 MCP tool 级覆盖

### 2. 更薄的外部 `tool_reference` 协议

当前 runtime 已经有 automatic expansion，但对外协议仍是 OpenFang 当前实现，不是完整的 Anthropic-style 薄 reference。

### 3. builtin 的精细化 deferred rollout

当前 builtin 默认仍全部 visible，没有进入这一轮 deferred 策略。

## 当前代码对应关系

### skill defer_loading 解析与归一化

- [lib.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-skills/src/lib.rs)
- [openclaw_compat.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-skills/src/openclaw_compat.rs)
- [registry.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-skills/src/registry.rs)

### kernel 侧 tool surface 与 skills section 过滤

- [kernel.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-kernel/src/kernel.rs)

### prompt 结构与文案

- [prompt_builder.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-runtime/src/prompt_builder.rs)

### runtime tool execution 与 automatic expansion

- [tool_runner.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-runtime/src/tool_runner.rs)
- [agent_loop.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-runtime/src/agent_loop.rs)

## 本分支最终决策摘要

为了避免后续继续混淆，这里把已经确认的结论压成最终版：

1. `tool_search` 是唯一 discovery 入口。
2. `tool_get_instructions` 保留，用于按需加载 guidance/manual。
3. skill 的 `defer_loading` 由 skill 顶层声明，系统默认值是 `false`。
4. 不做 agent 级 skill defer override。
5. bundled skills 统一显式 `defer_loading: true`。
6. MCP deferred 配置方向已确认，但本分支还没实现。
7. prompt 只做三件事：
   - 告诉模型当前哪些 tools 可直接调用
   - 告诉模型当前有哪些能力域存在
   - 告诉模型当能力没直接可见时如何 discovery

这就是这个分支目前为止的真实方案边界。
