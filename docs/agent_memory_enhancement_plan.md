# 技术方案：增强 Agent 长期记忆、动态记忆上下文与记忆发现能力

## 1. 背景

OpenFang 原本已经具备三类和“记忆”相关的能力：

1. 基于共享 KV 存储的长期记忆工具：`memory_store` / `memory_recall`
2. 工作区级约束文件：`MEMORY.md`
3. 每日/阶段性摘要沉淀：工作区 `memory/` 目录与共享 KV 中的 `session_*` 条目

但在改造前，系统存在三个实际问题：

1. `MEMORY.md` 会在 Scaffold 阶段生成，但不会进入每轮对话的 system prompt。
2. Prompt 只弱提示“可以使用记忆工具”，没有清晰的记忆调用协议，模型经常跳过 recall/store。
3. 记忆工具只有精确 key 的 `memory_recall`，模型如果不知道 key 名，只能盲猜，召回成功率不稳定。

这意味着系统虽然“有记忆能力”，但模型在真实对话里既看不到长期记忆协议，也不容易稳定地找到已有记忆。

## 2. 最终目标

这次改造不是只补一个 `MEMORY.md` 注入点，而是把长期记忆链路补齐到“可实际工作”的状态：

1. 让 `MEMORY.md` 真实进入 prompt，成为长期操作协议和稳定约束的一部分。
2. 让模型在每轮对话前自动拿到一小段动态记忆上下文，而不是完全依赖临场 recall。
3. 提供“先发现 key、再精确 recall”的能力，降低记忆命名猜错导致的失效。
4. 保持改动集中在 prompt、agent loop、memory tool 和兼容层，不重做整个 memory substrate。

## 3. 实际落地范围

最终代码实现比最初 MVP 更完整，包含以下几层。

### 3.1. 静态长期记忆注入：`MEMORY.md` 进入 system prompt

在 `PromptContext` 中增加了 `memory_md` 字段，并在 system prompt 构建阶段把 `MEMORY.md` 作为 `Long-Term Memory` 小节注入。

- 变更文件：`crates/openfang-runtime/src/prompt_builder.rs`
- 关键点：
  - `PromptContext` 新增 `memory_md: Option<String>`
  - `build_system_prompt()` 在 `Full` 和 `Minimal` 两种模式下都尝试注入 `MEMORY.md`
  - 注入位置在 `USER.md` 之后，属于 workspace guidance sections 的一部分

这样 Agent 每轮推理时都能直接看到长期记忆协议，而不再依赖外部工具说明碰运气。

### 3.2. 占位 `MEMORY.md` 过滤

Scaffold 默认生成的占位版 `MEMORY.md` 信息量很低。如果无条件注入，会污染 prompt。

因此在 `prompt_builder` 中增加了占位内容识别逻辑：

- 当 `MEMORY.md` 仍是默认模板、没有被用户真正填写时，不注入该 section
- 当 `MEMORY.md` 被实际配置后，再作为长期约束进入 prompt

这保证了“有内容时生效，无内容时不占上下文”。

### 3.3. 动态记忆上下文注入

仅靠静态 `MEMORY.md` 还不够，因为它更像“协议”，不是具体历史事实。

为此，agent loop 在真正发起 LLM 请求前，会自动组装一条独立的动态记忆上下文消息，来源包括：

1. 最近的 `session_*` 摘要 KV
2. 本轮已经召回的 memory fragments

实现方式：

- 变更文件：`crates/openfang-runtime/src/agent_loop.rs`
- 配套格式化逻辑：`crates/openfang-runtime/src/prompt_builder.rs`
- 关键点：
  - `load_recent_session_summaries()` 从共享 KV 中抓取最近的 session 摘要
  - `format_recalled_memory_fragments()` 对 recalled memories 做去重和截断
  - `build_memory_context_message()` 将两者组装为一条可直接注入的消息
  - 这条消息会被 prepend 到本轮对话消息序列中

这一步解决了“模型即使不主动 recall，也完全没有历史上下文”的问题。

## 4. 新增记忆发现能力：`memory_list`

原始方案只依赖 `memory_recall(key)`，但它要求模型先猜中 key，实际不够稳。

因此实现里新增了 `memory_list`，用于先发现已有记忆 key，再决定是否进一步 recall。

### 4.1. 能力定义

- Tool 名称：`memory_list`
- 用途：
  - 按 prefix / query 列出已有 memory keys
  - 可限制返回数量
  - 可选择是否返回 value

### 4.2. 实现位置

- `crates/openfang-runtime/src/kernel_handle.rs`
  - `KernelHandle` trait 新增 `memory_list(...)`
- `crates/openfang-kernel/src/kernel.rs`
  - 基于共享 memory substrate 的 KV 列表实现
- `crates/openfang-runtime/src/tool_runner.rs`
  - 注册 `memory_list` 为内置工具
  - 实现输入解析与 JSON 输出

### 4.3. 协议变化

prompt 中的 memory guidance 也同步更新：

- 当 key 明确时，优先 `memory_recall`
- 当 key 不明确时，先 `memory_list` 看候选 key，再决定是否 recall

这使记忆调用从“纯猜 key”升级为“两段式发现 + 精确读取”。

## 5. 兼容层与权限面修正

为了让新能力真正可用，还同步改了工具兼容层、Agent 工具配置和脚手架提示。

### 5.1. 旧工具名兼容

- 变更文件：`crates/openfang-types/src/tool_compat.rs`
- 兼容策略：
  - `memory_search` 映射到 `memory_list`

这样旧配置或迁移数据里出现 `memory_search` 时，不会直接失效。

### 5.2. Agent 工具配置更新

- 变更文件：`crates/openfang-types/src/agent.rs`
- 更新内容：
  - `Assist` 模式的只读工具集合加入 `memory_list`
  - `ToolProfile::Messaging` / `ToolProfile::Automation` 加入 `memory_list`

### 5.3. Wizard / Scaffold 提示更新

- 变更文件：`crates/openfang-kernel/src/wizard.rs`
- 更新内容：
  - 记忆能力不再只包含 `memory_store` / `memory_recall`
  - 新增 `memory_list`
  - 引导文案明确“key 不确定时先 discovery，再 recall”

### 5.4. 迁移测试更新

- 变更文件：`crates/openfang-migrate/src/openclaw.rs`
- 更新内容：
  - 测试断言改为 `memory_search -> memory_list`

## 6. Kernel 侧接线

`MEMORY.md` 的读取和传递在 kernel 中补齐到了所有 prompt 组装入口。

- 变更文件：`crates/openfang-kernel/src/kernel.rs`
- 关键点：
  - 在构建 `PromptContext` 的两个主要路径中都读取工作区 `MEMORY.md`
  - 通过 `read_identity_file(..., "MEMORY.md")` 注入 `memory_md`

这样不管走哪条主要执行链路，`MEMORY.md` 都能进到 runtime prompt builder。

## 7. 历史裁剪问题与修复

在 live integration 测试中发现了一个真实问题：

- 动态记忆上下文虽然被 prepend 到消息列表前面
- 但长会话下历史裁剪逻辑会先从前面裁掉旧消息
- 结果导致刚插进去的 memory context 也被一起裁掉

为此在 `agent_loop` 中增加了带保留槽位的裁剪逻辑：

- `trim_messages_for_prepended_context(messages, reserved_slots)`

含义是：

- 先为 prepend 的上下文消息预留位置
- 再裁剪普通历史消息
- 确保动态记忆上下文不会被自己家的裁剪逻辑吞掉

这部分不在原始 MVP 文档里，但属于 live 验证中暴露出的必须修复项。

## 8. 当前系统行为

改造完成后，系统的记忆行为变为：

1. 每轮请求都会尝试把工作区 `MEMORY.md` 注入 system prompt。
2. 若 `MEMORY.md` 仍是占位模板，则不注入，避免浪费上下文。
3. 每轮请求前，系统会自动注入少量动态记忆上下文：
   - 最近 session 摘要
   - 已召回的 memory fragments
4. 模型可在不知道精确 key 时先使用 `memory_list` 发现候选 key。
5. 发现 key 后，再使用 `memory_recall` 做精确读取。

需要明确的是：

- 系统并不会把整个 `memory/` 目录全部自动塞进 prompt。
- 当用户问到“昨天/上周某天具体聊了什么”这类强日期定位问题时，Agent 仍应使用 `file_read` / `file_search` 去读工作区 `memory/` 下对应文件。

## 9. 建议的 `MEMORY.md` 内容职责

在当前实现下，`MEMORY.md` 更适合承载以下内容：

1. 长期稳定的行为协议
2. 记忆使用规则
3. 稳定用户偏好或项目长期约束

不建议把大量短期事实、一次性对话结论堆进 `MEMORY.md`，因为这些内容更适合：

- 存入 KV memory
- 或沉淀到 `memory/*.md` / `session_*` 摘要

建议示例：

```markdown
# Long-Term Memory

## Memory Protocol
- 当任务依赖历史背景、项目状态或用户长期偏好时，先检查记忆，而不是直接猜测。
- 如果你不知道准确 key，先使用 `memory_list` 查看候选 key，再使用 `memory_recall` 精确读取。
- 在确认了稳定偏好、项目关键决策或长期状态后，使用 `memory_store` 沉淀。

## Stable Preferences
- 默认使用中文交流，但代码、变量名和技术术语保留英文。
- 做高风险修改前先审查影响面，不要盲目批量改写。

## Durable Project Context
- 记录项目长期约束、架构边界、固定工作流和重要协作习惯。
```

## 10. 变更文件清单

本次实现实际涉及以下文件：

- `crates/openfang-runtime/src/prompt_builder.rs`
- `crates/openfang-runtime/src/agent_loop.rs`
- `crates/openfang-runtime/src/tool_runner.rs`
- `crates/openfang-runtime/src/kernel_handle.rs`
- `crates/openfang-kernel/src/kernel.rs`
- `crates/openfang-kernel/src/wizard.rs`
- `crates/openfang-types/src/agent.rs`
- `crates/openfang-types/src/tool_compat.rs`
- `crates/openfang-migrate/src/openclaw.rs`

## 11. 验证结果

本次改造已完成以下验证：

- `cargo build --workspace --lib`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`

并进行了 live integration 验证：

- daemon 可正常启动
- `/api/health` 与 `/api/agents` 可正常返回
- 真实 `/api/agents/{id}/message` 请求可进入 runtime
- 通过 `llm.log` 确认 `MEMORY.md` 和 `memory_list` 已进入实际运行链路

外部 LLM completion 在验证时受上游 provider 地域/鉴权限制，没有完成成功生成；但这不影响对本次 prompt wiring、tool exposure 与消息拼装链路的验证。

## 12. 结论

最初的提案只解决了“`MEMORY.md` 没有进入 prompt”的断层。当前实际实现已经把方案扩展为完整的长期记忆增强链路：

1. `MEMORY.md` 静态注入
2. 动态记忆上下文预注入
3. `memory_list` 记忆发现能力
4. 兼容层、工具暴露面和脚手架提示同步更新
5. live 验证中发现并修复历史裁剪对记忆上下文的破坏

因此，这份文档应被视为“已实现设计说明”，而不是“待实现的最小方案草案”。
