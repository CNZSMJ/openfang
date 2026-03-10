# ADR-0001：OpenClaw 兼容的 Prompt 文件契约与注入顺序

## 状态

已接受，第一阶段已落地

## 日期

2026-03-10

## 背景

OpenFang 当前的 prompt 组装方式存在三个核心问题：

- 用户可编辑文件与 prompt section 不是一一对应，系统会把多个文件混成一个大段
- 同一个文件可能被正文注入一次，又在工作区摘要里预览一次，造成重复和语义漂移
- 平台规则、Agent 规则、用户信息混在一起，不利于用户理解，也不利于大模型注意力聚焦

在审阅 OpenClaw 的 prompt 实现后，得到两个重要结论：

1. 工作区文件最好保持稳定的文件契约，不要让系统自动重解释用户写在文件里的语义
2. 大模型对注入顺序敏感。高约束、高代价出错的内容应该更靠前，尤其是平台规则和 `AGENTS.md`

OpenClaw 的源码体现了这一点：

- workspace 文件的默认加载顺序是 `AGENTS.md -> SOUL.md -> TOOLS.md -> IDENTITY.md -> USER.md -> HEARTBEAT.md -> BOOTSTRAP.md -> MEMORY`，见 [workspace.ts](../../../claw/openclaw/src/agents/workspace.ts)
- 被加载的文件会在 `Project Context` 中按文件路径原样注入，见 [system-prompt.ts](../../../claw/openclaw/src/agents/system-prompt.ts)
- `SOUL.md` 有额外提示要求模型体现其人格与语气，但文件内容本身不被系统拆义

## 决策

OpenFang 采用如下原则重构 prompt 组装：

1. 保持与 OpenClaw 尽可能一致的工作区文件契约
2. 用户可编辑文件与 prompt section 尽量一一对应
3. 平台、Agent、用户、Memory 四类语义必须分层，不能混写
4. 平台规则始终前置，工作区文件按固定顺序注入
5. 暂不引入 `PLAYBOOK` 与 `SELF_STATE`
6. 暂不允许 agent 自动改写核心人格或规则文件

## 本次已落地范围

截至 2026-03-10，本 ADR 的第一阶段已经在代码中落地，范围包括：

1. `prompt_builder` 已重构为显式 section 组装，而不是旧式混合人格块
2. 引入显式 `PromptMode::{Full, Minimal}`
3. 工作区文件主干顺序固定为 `AGENTS.md -> SOUL.md -> IDENTITY.md -> USER.md -> TOOLS.md`
4. `workspace_context` 改为运行时摘要，不再重复预览 `SOUL.md` / `AGENTS.md` / `IDENTITY.md` 等文件
5. 新建 agent 改为通过 scaffold 生成 `SOUL.md`、`AGENTS.md`、`IDENTITY.md`、`USER.md`、`TOOLS.md`、`MEMORY.md`、`BOOTSTRAP.md`
6. 内置 `assistant` 模板已迁移为“薄 `system_prompt` + companion markdown files”
7. 记忆仍采用 `Memory Recall Protocol + 按需召回`，不把 `MEMORY.md` 全文常驻注入

关键实现入口：

- [crates/openfang-runtime/src/prompt_builder.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-runtime/src/prompt_builder.rs)
- [crates/openfang-kernel/src/kernel.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-kernel/src/kernel.rs)
- [crates/openfang-cli/src/templates.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-cli/src/templates.rs)
- [crates/openfang-api/src/routes.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-api/src/routes.rs)
- [agents/assistant/agent.toml](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/agents/assistant/agent.toml)

## Ownership 定义

### 平台

由 runtime 维护的内容，包括：

- 安全边界
- 工具可用性与调用契约
- channel / sandbox / runtime / messaging 规则
- 记忆检索协议

### Agent

描述 agent 自身的内容，包括：

- 身份
- 人格
- 行为规则
- 自治运行规则
- 初始化规则

### 用户

描述用户与用户环境的内容，包括：

- 用户画像与关系上下文
- 用户提供的工具/环境说明

### Memory

描述长期记忆与近期经历的内容，包括：

- 长期稳定记忆摘要
- 近期事件与时间性记忆
- 需要按需召回的历史片段

## 文件契约

每个文件只承担一种明确语义：

| 归属 | 文件 | 唯一职责 |
|---|---|---|
| Agent | `AGENTS.md` | agent 的行为规则、边界、协作方式、红线 |
| Agent | `SOUL.md` | agent 的人格、语气、价值观、使命感 |
| Agent | `IDENTITY.md` | agent 的身份元数据、角色标签、稳定自我定义 |
| 用户 | `USER.md` | 用户身份、称呼、偏好、关系上下文 |
| 用户 | `TOOLS.md` | 用户提供的环境说明、项目约定、外部工具习惯 |
| Memory | `MEMORY.md` | 长期稳定记忆摘要 |
| Memory | `memory/*.md` | 近期事件、日记式经历、时间性记忆 |
| Agent | `HEARTBEAT.md` | agent 在自治模式下的周期性检查规则 |
| Agent | `BOOTSTRAP.md` / `BOOT.md` | 首次运行或启动阶段的初始化、自检和引导 |

约束如下：

- 不从 `AGENTS.md` 自动拆出平台规则或学习层
- 不从 `SOUL.md` 自动拆出结构化人格字段
- 不把多个文件重新拼成一个混合人格块
- 不再依赖工作区文件预览去表达文件语义

## 注入顺序

默认主 agent 的注入顺序如下：

1. 平台固定层：`Core OS Policy`
2. 平台运行层：工具、权限、channel、sandbox、runtime 契约
3. Agent：`AGENTS.md`
4. Agent：`SOUL.md`
5. Agent：`IDENTITY.md`
6. 用户：`USER.md`
7. 用户：`TOOLS.md`
8. Memory：`Memory Recall Protocol`
9. Memory：按需检索 `MEMORY.md` 和 `memory/*.md`
10. Agent：`HEARTBEAT.md`，仅自治场景注入
11. Agent：`BOOTSTRAP.md` / `BOOT.md`，仅首次运行或启动场景注入

当前代码中的 `full` 模式实现与上面保持一致，只是在平台层内部继续保留：

- 当前日期
- 工具列表
- skills 摘要
- MCP 摘要
- workspace 运行时摘要
- channel 信息
- peer agents
- safety
- operational guidelines

### 顺序理由

- 平台规则必须先于所有工作区文件，避免工具、消息、权限和安全约束被后续文本稀释
- `AGENTS.md` 必须在工作区文件中最前，因为它承载高优先级行为规则和红线
- `SOUL.md` 紧随 `AGENTS.md`，让人格建立在行为约束之内，而不是反过来压过规则
- `IDENTITY.md` 位于 `SOUL.md` 之后，因为它更偏稳定标签，而不是高权重行为约束
- `USER.md` 和 `TOOLS.md` 在后，让 agent 先知道自己如何做事，再知道面对谁、处于什么环境
- `MEMORY.md` 和 `memory/*.md` 默认不全文常驻，以检索召回优先，避免污染主干注意力

## Memory 策略

记忆是本次设计中的唯一例外。

虽然文件契约仍然保持清晰，并且 ownership 属于 `Memory`：

- `MEMORY.md`
- `memory/*.md`

但它们默认不直接全文注入常驻 prompt，而是采用：

1. 固定的 `Memory Recall Protocol`
2. 按需检索
3. 按需读取片段
4. 在需要时注入检索结果

原因是长期记忆天然属于召回语义，不适合每轮全文常驻。

## 暂缓项

本 ADR 明确暂缓以下内容：

- `PLAYBOOK.md`
- `SELF_STATE.md`
- 自动晋升的自进化层
- agent 自动改写 `AGENTS.md` / `SOUL.md` / `IDENTITY.md` / `USER.md` / `TOOLS.md`

当前阶段只做：

- 文件契约清晰化
- 注入顺序稳定化
- 平台/Agent/用户/Memory ownership 分层
- 记忆检索化

## 需要落实的改动

### 1. 重构 prompt builder

把当前混合注入方式改为显式 section：

- 删除当前“混合人格块”式的注入
- 改为按文件独立 section 注入
- 去掉同一文件被正文与 workspace preview 双重注入的情况

### 2. 明确平台层与文件层边界

将以下内容明确保留在平台层，而不是从工作区文件中提取：

- 安全规则
- 工具协议
- channel / messaging 规则
- sandbox 规则
- runtime 信息

### 3. 调整工作区文件注入顺序

工作区文件按如下固定顺序处理：

1. `AGENTS.md`
2. `SOUL.md`
3. `IDENTITY.md`
4. `USER.md`
5. `TOOLS.md`
6. `HEARTBEAT.md`（条件）
7. `BOOTSTRAP.md`（条件）
8. `MEMORY.md` / `memory/*.md`（检索）

### 4. 取消工作区预览式上下文

不再使用“把文件截断后塞进 workspace summary”的方式表达文件语义。

工作区摘要只保留真正的运行环境信息，例如：

- 项目类型
- 构建命令
- 测试命令
- 关键入口
- 仓库约束

### 5. 引入显式模式裁剪

参考 OpenClaw，保留主 agent / subagent 的差异化注入策略。

至少区分：

- full
- minimal

在 minimal 模式下，缩减不必要的文件与平台 section。

### 6. 引入文件级预算与截断

对工作区文件建立：

- 单文件字符上限
- 总注入上限
- 截断提示

避免某个文件过长吞噬主要注意力预算。

### 当前实现细化

第一阶段没有引入复杂的全局 token budget allocator，但已经做了三项保守收敛：

1. 提高 `AGENTS.md` / `SOUL.md` / `IDENTITY.md` / `TOOLS.md` 的单文件预算，避免核心语义被中途截断
2. 对 `SOUL.md` 及其它工作区文件做 code block stripping，降低把示例代码误当成执行指令的风险
3. 当 `USER.md` / `TOOLS.md` 仍然只是默认占位模板时，跳过注入，避免空信息占用注意力

这三项都已在 [crates/openfang-runtime/src/prompt_builder.rs](/Users/jiahaohuang/workspace_ai/openfang-0.1.0/feature-cyber-soul/crates/openfang-runtime/src/prompt_builder.rs) 实现并有测试覆盖。

## Prompt Mode

当前阶段引入两种显式 prompt mode：

- `full`
- `minimal`

暂不引入 `none`。

### `full`

适用场景：

- 主 agent 的常规对话
- 需要完整人格与上下文的长会话
- 首次运行与 onboarding

默认包含：

1. 平台固定层
2. 平台运行层
3. `AGENTS.md`
4. `SOUL.md`
5. `IDENTITY.md`
6. `USER.md`
7. `TOOLS.md`
8. `Memory Recall Protocol`
9. 按需 memory 检索结果
10. `HEARTBEAT.md`（条件）
11. `BOOTSTRAP.md` / `BOOT.md`（条件）

### `minimal`

适用场景：

- subagent
- 短生命周期执行型 agent
- heartbeat 场景
- 压缩后恢复时需要快速重建关键约束的场景

默认包含：

1. 平台固定层
2. 平台运行层
3. `AGENTS.md`
4. `SOUL.md`
5. `TOOLS.md`
6. 必要时的 `Memory Recall Protocol`

默认省略：

- `IDENTITY.md`
- `USER.md`
- 常规 memory 注入
- `BOOTSTRAP.md`
- 非当前任务强相关的辅助 section

### Mode 选择规则

当前建议规则：

- 主 agent 默认使用 `full`
- subagent 默认使用 `minimal`
- heartbeat runner 使用 `minimal`
- `BOOTSTRAP.md` / `BOOT.md` 不是单独 mode，而是在 `full` 下按条件附加

### 设计理由

- `full` 用于保持完整人格、关系和环境上下文
- `minimal` 用于在较小 token 预算下优先保留行为约束与执行能力
- `minimal` 仍保留 `SOUL.md`，以维持最小人格连续性，避免子 agent 退化为纯工具体
- `AGENTS.md` 在 `minimal` 中仍优先于 `SOUL.md`，确保规则不会被人格文本覆盖

## 影响

正面影响：

- 用户可编辑文件与 prompt section 的关系更清楚
- 大模型更容易先看到高优先级行为约束
- 人格文件仍然保持强影响力，但不会压过规则文件
- 与 OpenClaw 的文件契约更接近，迁移和兼容成本更低
- 记忆从常驻背景改成按需召回，更利于 token 控制

成本与约束：

- 需要重构现有 prompt builder 的 section 结构
- 需要重新定义工作区摘要与文件注入的职责边界
- 需要引入文件级预算控制
- 需要重新校验主 agent 与 subagent 的 prompt 差异

## 实施结果

### 已完成

- `PromptMode::{Full, Minimal}` 已落地
- `assistant` 的 `agent.toml` 已降薄
- `assistant` 的 `SOUL.md`、`AGENTS.md`、`IDENTITY.md`、`BOOTSTRAP.md` 已按新职责重写
- 新建 agent 的 wizard / agents 页面不再只生成大段 `system_prompt`，而是发送 scaffold
- CLI / API / bundled template 安装路径都会加载 companion markdown scaffold
- `USER.md` / `TOOLS.md` 的空壳模板默认不会污染 prompt
- 平台规则中“文件里出现命令就执行”的高风险描述已收紧

### 尚未完成

- 尚未引入更精细的全局 prompt budget ranking
- 尚未做 `PLAYBOOK` / `SELF_STATE`
- `assistant` 的 `USER.md`、`TOOLS.md`、`MEMORY.md` 仍然偏模板化，后续可继续充实
- 其它内置 agent 还未全部迁移到 `assistant` 这一套内容质量标准

## 成功标准

当满足以下条件时，本决策视为生效：

- 用户能明确知道每个文件各自控制什么
- 每个用户可编辑文件在 prompt 中都有唯一语义位置
- `AGENTS.md` 在工作区文件中具有最高行为优先级
- `SOUL.md` 能稳定影响语气和人格，但不会覆盖行为红线
- `MEMORY.md` 与 `memory/*.md` 作为 `Memory` 层，以检索召回为主，而不是全文常驻
- 同一文件不再被正文与工作区预览重复注入

当前状态：

- 前五条已基本满足
- “文件级预算与排序” 已完成第一阶段
- “其它内置 agent 模板内容全部收敛” 仍待继续推进

## 参考

- [custom-system-prompt-analysis.md](custom-system-prompt-analysis.md)
- [agent-context-evolution-design.md](agent-context-evolution-design.md)
- [workspace.ts](../../../claw/openclaw/src/agents/workspace.ts)
- [system-prompt.ts](../../../claw/openclaw/src/agents/system-prompt.ts)
- [post-compaction-context.ts](../../../claw/openclaw/src/auto-reply/reply/post-compaction-context.ts)
