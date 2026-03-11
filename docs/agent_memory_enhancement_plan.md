# 技术方案：通过注入 `MEMORY.md` 增强 Agent 长期记忆与主动记忆能力

## 1. 背景 (Background)

OpenFang 的 Agent 系统目前拥有一套基于 Key-Value 分布式存储（底层为 SQLite `memory.db`）的共享记忆系统，Agent 可以通过 `memory_store` 和 `memory_recall` 两个工具进行交互。同时，系统后台会静默记录每日的交互流水账，保存在工作区的 `memory/` 目录下（例如 `memory/2026-03-12.md`）。

然而，当前系统的记忆表现存在明显的“断层”：
1. **Agent 无法感知历史流水账**：每日生成的 `memory/*.md` 文件并没有在对话开始前喂给大模型，导致 Agent 记不住过往发生的事件。
2. **缺乏主动记忆规范**：系统内置的 System Prompt（位于 `crates/openfang-runtime/src/prompt_builder.rs`）仅仅简单告知了工具的存在，并没有强有力的规则（Protocol）要求模型去主动搜索或沉淀知识，大模型的惰性导致其往往会忽略使用这些记忆库工具。
3. **`MEMORY.md` 配置被遗漏加载**：虽然系统在 Agent 初始化 Scaffold 阶段会生成 `MEMORY.md`，但在构建每轮回答的 Prompt 时，`PromptContext` 读取了 `SOUL.md`、`USER.md` 等绝大多数配置文档，唯独漏掉了对 `MEMORY.md` 文件的读取与注入。

## 2. 目的 (Objective)

本方案的目的是彻底解决 Agent 的“健忘”问题，使其能够遵循用户的个人偏好，并学会主动查询与沉淀上下文：
1. **修复 `MEMORY.md` 的断层**：通过系统底层代码的修改，将工作区下的 `MEMORY.md` 正确加载，并注入到大语言模型的核心 System Prompt 中。
2. **制定标准的记忆调用协议 (Memory Protocol)**：抛弃原有的仅仅罗列信息的 `MEMORY.md`，重新设计具有强约束力和行动纲领性质的 `MEMORY.md` 内容。充分发挥 `memory_store` 和 `memory_recall` 工具，以及 `memory/*.md` 日志目录的作用。

## 3. 设计思路 (Design Approach)

为了避免对代码做大规模侵入并保持原有的简洁，修改点仅集中在 `PromptContext` 的传递与结构拼接。

1. **载体**：`MEMORY.md` 是最理想的长期工作约束载体。它不会每天变化，适合挂载**操作规程**和**核心准则**。
2. **组装顺序**：在 `crates/openfang-runtime/src/prompt_builder.rs` 拼接 System Prompt 时，将 `MEMORY.md` 注入到 `Section 9+ — Workspace guidance sections` 中。最佳位置是紧随 `USER.md`（用户偏好）之后，内置的 `Memory Recall Protocol` 小节之前。这样 Agent 能将两部分融会贯通。
3. **内容重构**：新的 `MEMORY.md` 将明确告诉 Agent 什么时候需要强制执行 `memory_store`（例如需求讨论结束后），什么时候强制调用 `memory_recall`（例如面对需要前置历史背景的复质任务），以及当被问及昨天或更久远的聊天记录时，应该去读取 `memory/` 目录。

## 4. 代码变更细节 (Implementation Details)

为保证与现有代码风格命名完全一致，修改规划如下：

### 4.1. 修改 `crates/openfang-runtime/src/prompt_builder.rs`

- **修改 `PromptContext` 结构体**（约 61 行附近）：
  增加 `memory_md` 字段映射：
  ```rust
  /// MEMORY.md content (long-term memory and memory operational protocol).
  pub memory_md: Option<String>,
  ```

- **修改 `build_system_prompt` 函数**（约 143 行附近）：
  在拼接 `USER.md` 的代码块下方，注入 `MEMORY.md` 的内容（针对 `Full` 和 `Minimal` 两种模式均需确保被恰当处理，因为部分子 Agent 也需要遵守记忆操作协议）。
  ```rust
  if let Some(section) =
      build_workspace_file_section("Long-Term Memory", "MEMORY.md", ctx.memory_md.as_deref(), 2400)
  {
      sections.push(section);
  }
  ```

### 4.2. 修改 `crates/openfang-kernel/src/kernel.rs`

- **修改 `PromptContext` 组装逻辑**（约 2460 行附近，`let prompt_ctx = PromptContext { ... }` 内部）：
  映射读取本地工作区的 `MEMORY.md`：
  ```rust
  memory_md: manifest
      .workspace
      .as_ref()
      .and_then(|w| read_identity_file(w, "MEMORY.md")),
  ```

## 5. `MEMORY.md` 的重构设计 

忽略原有的 `MEMORY.md`，为充分发挥 OpenFang 的系统记忆能力，建议将其重构为下面这套高度规整的结构。请将如下内容覆盖写入至您的目标 Agent 的 `MEMORY.md` ( e.g. `~/.openfang/workspaces/assistant/MEMORY.md` )。

```markdown
# Long-Term Memory & Operation Protocol

你运行在 OpenFang 系统中。系统为你提供了多个“脑区”，你必须**严格遵循以下协议**来记忆或回顾信息，克服大语言模型固有的“惰性响应”：

## 1. 动态记忆库协议 (`memory_store` / `memory_recall`)
这才是你真正的跨会话持久共享大脑，请将其视为最重要的状态存储器。
- **强制 Recall 机制**：当我要求你推进一个已有项目、讨论历史技术方案，或进行复杂多阶段任务时，**绝对不要凭空猜测**！请优先静默调用 `memory_recall` 提取相关 Key 的状态。
- **强制 Store 机制**：在完成技术选型讨论、项目重大决策、或提取出我的隐性偏好（如代码风格要求）后，必须调用 `memory_store` 以 Key-Value 格式存储起来！
- **命名规范**：Key 的命名必须使用 dot-notation （如 `pref.code_style`, `project.foo.arch`, `session.status`），保持语义清晰。

## 2. 流水账日记阅读法 (`memory/` 目录)
- 系统底层每天都会替你写流水账，存放在你工作目录的子文件夹 `memory/` 里（如 `memory/2026-03-12.md`）。
- 这些日记内容只有几百字的简略摘要。**注意：系统并没有把这些日记直接放入你的本次上下文里！你当前是看不见它们的。**
- **操作规则**：如果我明确问你“昨天我们聊了什么”、“刚才那个定时任务我上周二是怎么要求你的”，你应该调用 `file_read` 或 `file_search`，去主动查阅 `memory/` 下对应日期的 Markdown 文件。

## 3. 沟通边界和强制偏好
- 必须保持犀利、机智且真诚的沟通风格（sharp, witty, non-corporate），不要像一个刻板的客服。
- 我们使用中文交流，但代码、变量和技术专有名词请保留英文。
- 在修改我的重要代码文件，或进行有高风险、大规模的文件写入前，可以主动在临时目录备份或提示风险。
- **绝不允许**主动或静默运行类似 `cargo fmt` 等代码格式化指令，除非我直接、明确地要求。
```
