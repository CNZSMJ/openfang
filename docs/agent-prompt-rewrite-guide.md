# Agent Prompt 改写指导说明

## 目的

本文档用于指导大模型在 OpenFang 新的 prompt 组装逻辑下，系统性改写指定 agent 的以下三类内容：

1. 仓库中的模板文件
2. agent 创建时生成的初始 workspace 文件
3. 已经生成并落地的 agent workspace 文件

目标不是“把 prompt 写得更长”，而是让每一块内容回到正确的位置，减少重复、避免冲突，并让 `full` / `minimal` 两种模式都能稳定工作。

## 适用范围

当你要改写某个 agent 时，至少要同时审查以下两类来源：

- 仓库模板：`agents/<agent-name>/agent.toml`
- workspace 文件：`AGENTS.md`、`SOUL.md`、`IDENTITY.md`、`USER.md`、`TOOLS.md`、`MEMORY.md`、`BOOTSTRAP.md`、`HEARTBEAT.md`

如果目标不只是修改单个现有 agent，而是修正系统默认生成结果，还必须审查：

- 初始 workspace 文件生成逻辑：
  [crates/openfang-kernel/src/kernel.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-kernel/src/kernel.rs#L291)
- 模板 scaffold 生成逻辑：
  [crates/openfang-api/static/js/pages/wizard.js](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-api/static/js/pages/wizard.js)
  [crates/openfang-api/static/js/pages/agents.js](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-api/static/js/pages/agents.js)
- CLI / API 模板加载逻辑：
  [crates/openfang-cli/src/templates.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-cli/src/templates.rs)
  [crates/openfang-api/src/routes.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-api/src/routes.rs)

## 先理解系统现在如何组装 prompt

### `full` 模式

当前主 agent 默认会按以下顺序组装系统 prompt：

1. `agent.toml` 中的 `model.system_prompt`
2. 当前日期
3. 平台工具调用规则
4. 工具列表
5. skills 摘要
6. MCP 摘要
7. workspace 运行时上下文
8. channel 信息
9. peer agents
10. safety
11. 平台 operational guidelines
12. `AGENTS.md`
13. `SOUL.md`
14. `IDENTITY.md`
15. `USER.md`
16. `TOOLS.md`
17. Memory Recall Protocol
18. `HEARTBEAT.md`（条件）
19. `BOOTSTRAP.md`（条件）

相关实现见：

- [crates/openfang-runtime/src/prompt_builder.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-runtime/src/prompt_builder.rs#L37)
- [docs/adr-0001-agent-prompt-architecture.md](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/docs/adr-0001-agent-prompt-architecture.md#L100)

### `minimal` 模式

subagent 和轻量执行场景会省略一部分文件：

- 仍保留：`agent.toml` 的基础 prompt、平台层、`AGENTS.md`、`SOUL.md`、`TOOLS.md`、Memory Recall Protocol
- 默认省略：`IDENTITY.md`、`USER.md`、`BOOTSTRAP.md`、常规 peer/context 扩展

这意味着：

- `AGENTS.md` 必须能单独约束行为
- `SOUL.md` 必须能在缺少 `IDENTITY.md` / `USER.md` 时仍保持人格连续性
- `TOOLS.md` 只能放环境/约定，不能承担核心人格或规则
- `USER.md` / `TOOLS.md` 如果还只是空壳模板，运行时可能会被跳过，不应把关键信息只写成占位符

### 当前已实现的额外规则

当前 builder 已经落地以下保守规则：

1. `AGENTS.md` / `SOUL.md` 的预算已提高，避免把核心段落从中间截断
2. `USER.md` 如果只有空字段模板，不注入
3. `TOOLS.md` 如果仍然等于默认通用模板，不注入
4. `SOUL.md` 和其它工作区文件中的 code block 会在注入前被剥离
5. 平台层不会再因为 persona 或模板文件里出现命令示例就自动执行它们

这意味着改写内容时必须区分：

- “真实上下文”
- “模板提示”

只有前者应该长期影响 prompt。

## 内容定位总表

改写时必须先按 ownership 拆分内容，再决定写到哪里。

| 文件/位置 | 应该承载什么 | 不应该承载什么 |
|---|---|---|
| `agents/<name>/agent.toml` 的 `model.system_prompt` | 最基础身份、职责边界、默认工作方式 | 工具清单、平台规则、memory 协议、长篇行为细则 |
| `AGENTS.md` | 该 agent 专属的行为规则、边界、优先级、委派策略、输出偏好 | 平台通用工具协议、安全总则、通用 shell/file/web 规则 |
| `SOUL.md` | 人格、语气、价值观、互动气质 | 工具调用步骤、流程 checklist、能力清单 |
| `IDENTITY.md` | 稳定身份标签、frontmatter、角色元数据、视觉风格 | 大段行为规则、用户偏好、工具说明 |
| `USER.md` | 用户身份、称呼、关系上下文、长期偏好 | agent 自我定义、系统规则、工具策略 |
| `TOOLS.md` | 用户环境、仓库约定、外部工具习惯、项目命令 | 平台通用 tool policy、人格和价值观 |
| `MEMORY.md` | 需要长期保留的稳定记忆摘要 | 每轮都变的临时任务状态、详细行为规则 |
| `BOOTSTRAP.md` | 首次交互或冷启动时的初始化流程 | 常驻行为规则、长期人格描述 |
| `HEARTBEAT.md` | 自治 agent 的周期检查项 | 普通对话风格、通用 onboarding |

## 改写总原则

### 1. 优先删重复，再补内容

如果某段内容已经被平台层稳定注入，就不要再放进 agent 文件。

高频重复内容包括：

- tool call behavior
- 安全总则
- memory_recall / memory_store 的平台协议
- 通用的 “act first, narrate second”
- 通用工具列表

### 2. `agent.toml` 要短，不要再承担整个系统 prompt

`model.system_prompt` 现在是整个系统 prompt 的第一个 section。它应当像“agent 的基础身份声明”，而不是“完整操作手册”。

推荐长度：

- 1 到 4 个短段落
- 尽量少于 800 英文词或对应长度

推荐结构：

1. 你是谁
2. 你负责什么
3. 默认怎么做决策
4. 什么时候交给别的 agent

当前仓库中的 `assistant` 已经按这个标准迁移，可作为基准参考：

- [agents/assistant/agent.toml](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/agents/assistant/agent.toml)
- [agents/assistant/AGENTS.md](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/agents/assistant/AGENTS.md)
- [agents/assistant/SOUL.md](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/agents/assistant/SOUL.md)
- [agents/assistant/IDENTITY.md](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/agents/assistant/IDENTITY.md)

### 3. `AGENTS.md` 比 `SOUL.md` 更偏规则

如果一段内容回答的是“这个 agent 应该怎么做事”，优先放 `AGENTS.md`。

如果一段内容回答的是“这个 agent 给人的感觉是什么”，优先放 `SOUL.md`。

### 4. `SOUL.md` 不是第二份 `AGENTS.md`

不要把下面内容写进 `SOUL.md`：

- “先读文件再写文件”
- “需要时调用 web_search”
- “不要解释工具调用”
- “发现风险先确认”

这些都属于平台层或 `AGENTS.md`，不是人格。

### 5. `TOOLS.md` 是环境说明，不是工具使用教程

`TOOLS.md` 应描述用户或项目环境，例如：

- 仓库语言/框架
- 常用构建命令
- 项目约定
- 特定外部系统的访问习惯

不要在 `TOOLS.md` 写这种内容：

- file_read / file_write 的一般规则
- 通用 web 工具策略
- memory_store 的使用原则

另外，若 `TOOLS.md` 只是默认模板，当前实现会直接跳过它的注入。因此：

- 该文件要么保持为空壳，占位即可
- 要么写入真实仓库/环境事实，让它真正值得进入 prompt

不要写“半模板半事实”的低信噪比内容。

## 针对三类对象的改写规则

### 一、仓库中的模板如何改

目标文件：

- `agents/<agent-name>/agent.toml`

改写步骤：

1. 读取 `model.system_prompt`
2. 给其中每一段打标签：
   - 基础身份
   - 领域能力
   - 行为规则
   - 人格语气
   - 用户关系
   - 工具/平台规则
3. 只保留以下内容在 `agent.toml`：
   - 基础身份
   - 职责范围
   - 高层默认策略
   - 专属委派策略
4. 将剩余适合下沉到 workspace 的内容记录为迁移建议：
   - 行为规则 -> `AGENTS.md`
   - 人格语气 -> `SOUL.md`
   - 身份元数据 -> `IDENTITY.md`
   - 用户关系 -> `USER.md`
   - 环境约定 -> `TOOLS.md`
5. 删除任何纯平台重复内容

改写判断标准：

- 读完 `agent.toml` 后，应该能知道“这是谁、负责什么、何时委派”
- 不应该再看到长工具列表、memory 使用说明、平台安全规则的重复副本

### 二、agent 创建时的初始版本如何改

目标代码：

- [crates/openfang-kernel/src/kernel.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-kernel/src/kernel.rs#L291)

这里定义的是新 agent 首次创建时自动写入 workspace 的模板内容。

当前已不是“只靠 kernel 默认文件模板”这一条链路，而是：

1. 前端 wizard / agents 页面生成 scaffold
2. API 将 scaffold 透传到 kernel
3. kernel 用 scaffold 优先写入文件
4. CLI / API 的 built-in template 创建路径会加载 `agents/<name>/` 下的 companion markdown files

改写原则：

1. 生成模板必须结构清晰、低重复、职责分离
2. 不要把平台层规则写死进所有 agent 的默认 `AGENTS.md`
3. 默认模板可以提供“首版可用内容”，但不能重新把所有平台规则和工具协议抄回去
4. 除非是自治 agent，否则不要生成多余文件

各文件建议：

- 默认 `AGENTS.md`
  - 可以预填 agent 专属工作方式
  - 说明此文件写行为规则、边界、委派策略
  - 不要预填一大段通用工具协议
- 默认 `SOUL.md`
  - 可以预填基础人格和语气
  - 但不要塞执行流程、工具策略、能力清单
- 默认 `IDENTITY.md`
  - 保持 frontmatter 模板
  - 不塞行为规则
- 默认 `USER.md`
  - 保持用户画像模板
- 默认 `TOOLS.md`
  - 保持环境说明模板
  - 可引导填写构建命令、仓库约束、外部系统
- 默认 `BOOTSTRAP.md`
  - 保留首次交互流程
  - 不要重复常驻规则
- 默认 `MEMORY.md`
  - 只保留长期记忆摘要说明

特别注意：

- 当前代码使用 `create_new(true)`，只在文件不存在时写入模板
- 这意味着修改模板代码只影响“未来新生成的 agent”
- 不会自动修复已经落地的旧 workspace
- 若默认 `USER.md` / `TOOLS.md` 仍未被填充，builder 可能会跳过它们，因此不要把关键规则只写进这两个模板文件

### 三、已生成的 agent 如何修改

目标路径通常是：

- `~/.openfang/workspaces/<agent-name>/`

改写步骤：

1. 读取该 agent 当前的：
   - `agent.toml`
   - `AGENTS.md`
   - `SOUL.md`
   - `IDENTITY.md`
   - `USER.md`
   - `TOOLS.md`
   - `BOOTSTRAP.md`
   - `MEMORY.md`
   - `HEARTBEAT.md`（如有）
2. 用“内容定位总表”重新分类每一段内容
3. 对重复内容做三选一：
   - 平台已覆盖 -> 删除
   - agent 专属行为 -> 移到 `AGENTS.md`
   - 人格表达 -> 移到 `SOUL.md`
4. 不要覆盖用户已积累在 `USER.md` / `MEMORY.md` 的真实数据
5. 如果 `IDENTITY.md` 仍是空模板，可以补齐稳定标签，但不要编造用户信息
6. 如果 `TOOLS.md` 为空，可以只补真正的环境说明，不要塞通用策略

已生成 agent 的修改原则比新模板更保守：

- 可以重组
- 可以删重复
- 可以补结构
- 不要丢失已存在的真实用户上下文

## 大模型执行改写时的输入要求

如果让大模型直接执行改写，至少应提供：

1. 目标 agent 名称
2. 当前 `agent.toml`
3. 当前 workspace 下全部相关 `.md` 文件
4. 当前 prompt 组装规则摘要
5. 输出范围：只给建议，还是直接改文件

## 大模型执行改写时的输出格式

推荐要求大模型输出以下内容：

1. 现状审查
2. 内容迁移表
3. 改写后的文件草案
4. 风险和保留项

推荐格式如下：

```markdown
## 审查结论
- 哪些内容重复
- 哪些内容放错位置
- 哪些内容缺失

## 迁移表
| 原位置 | 内容摘要 | 新位置 | 原因 |

## 改写后的文件
### agent.toml
...
### AGENTS.md
...

## 保留项
- USER.md 中哪些真实数据不应改
- MEMORY.md 中哪些内容只应整理不应删除
```

## 具体改写规则清单

### `agent.toml` 改写清单

- 保留 agent 的核心角色定义
- 保留专属任务边界
- 保留高层委派策略
- 删除通用工具列表
- 删除 memory 使用教程
- 删除平台通用安全规则
- 删除与 `AGENTS.md` 重复的执行细则
- 删除与 `SOUL.md` 重复的人格描述

### `AGENTS.md` 改写清单

- 写该 agent 独有的工作规则
- 写清楚默认处理方式和委派阈值
- 写输出风格偏好
- 写边界和红线
- 不要复制平台工具行为规则
- 不要写用户资料

### `SOUL.md` 改写清单

- 写人格、气质、价值观
- 写沟通风格
- 写面对不确定性时的姿态
- 不要写命令式工具流程
- 不要写长篇能力列表

### `IDENTITY.md` 改写清单

- 补 frontmatter
- 补稳定角色标签
- 补名称、archetype、vibe、greeting_style 等
- 不要写长篇规则

### `USER.md` 改写清单

- 只记录真实已知用户信息
- 不要臆造名字、偏好、背景
- 不要把 agent 的规则写进来

### `TOOLS.md` 改写清单

- 写项目/环境/工具约定
- 写常用命令和禁忌
- 写外部系统说明
- 不要复制平台工具清单

### `BOOTSTRAP.md` 改写清单

- 只写首次交互或冷启动流程
- 说明首次应收集哪些最小信息
- 说明如何在不打断任务的前提下完成 onboarding
- 不要写长期常驻规则

### `MEMORY.md` 改写清单

- 只放长期稳定记忆摘要
- 不要放每轮临时状态
- 不要把它当第二份 `USER.md`

## 面向大模型的禁止事项

执行改写时，不允许：

- 把平台规则重新塞回 `agent.toml`
- 把通用工具协议抄进每个 agent 的 `AGENTS.md`
- 用 `SOUL.md` 覆盖 `AGENTS.md` 的行为边界
- 在 `USER.md` 中编造用户信息
- 删除已有 workspace 中真实积累的偏好和记忆
- 把 `MEMORY.md` 改成操作手册

## 建议的改写提示词

下面这段可以直接给大模型作为改写任务说明：

```text
你正在为 OpenFang 改写一个指定 agent 的 prompt 相关文件。

你必须遵守以下系统事实：
1. `agent.toml` 的 `model.system_prompt` 是系统 prompt 的第一段，只应保留最基础身份、职责边界和高层策略。
2. 平台层会自动注入工具行为、安全、memory recall、工具列表、skills、runtime context 等内容，这些内容不要重复写入 agent 文件。
3. `AGENTS.md` 负责该 agent 专属的行为规则和边界。
4. `SOUL.md` 负责人格、语气、价值观，不负责工具流程。
5. `IDENTITY.md` 负责稳定身份元数据。
6. `USER.md` 只记录真实用户信息。
7. `TOOLS.md` 只记录环境和项目约定。
8. `BOOTSTRAP.md` 只负责首次交互流程。
9. `MEMORY.md` 只负责长期稳定记忆摘要。
10. subagent 的 `minimal` 模式可能不加载 `IDENTITY.md` 和 `USER.md`，所以 `AGENTS.md` 与 `SOUL.md` 必须独立成立。

你的任务：
1. 审查输入文件中的重复、冲突、错位内容。
2. 生成一张迁移表，说明每段内容应该移到哪里。
3. 输出改写后的 `agent.toml` 和各个 `.md` 文件草案。
4. 保留已有用户真实数据，不要编造。
5. 删除所有与平台层重复的通用规则。

输出顺序必须是：
- 审查结论
- 迁移表
- 改写后的文件
- 保留项与风险
```

## 推荐验收标准

一份改写结果只有在满足以下条件时才算合格：

- `agent.toml` 明显比旧版更短、更聚焦
- `AGENTS.md` 明显是“agent 专属规则”而不是平台规则副本
- `SOUL.md` 明显是人格文本，而不是 checklist
- `TOOLS.md` 明显是环境说明，而不是工具教程
- `USER.md` 和 `MEMORY.md` 没有被错误清空或臆造
- `full` 模式下信息完整
- `minimal` 模式下 `AGENTS.md` + `SOUL.md` 仍足以保持行为和人格

## 当前实现相关注意事项

- 当前实现读取的是 `BOOTSTRAP.md`，不是 `BOOT.md`
- 当前实现会读取 `TOOLS.md`
- 当前实现的新模板默认仍然偏通用，需要人工或后续代码改造来瘦身
- 修改 `generate_identity_files()` 只会影响未来新建 agent，不会回写已存在 workspace
