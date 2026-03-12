# 技术方案：Agent 长期记忆增强与 Prompt 编排落地说明

## 1. 文档定位

本文档描述的是当前分支中已经落地的长期记忆增强方案，以及围绕它形成的 prompt 编排与工具接线架构。

它不是最初的 MVP 草案，也不是“未来可能怎么做”的纯设想文档，而是：

1. 对本分支已实现行为的设计说明。
2. 对各层职责边界的整理，确保实现保持低耦合、高内聚。
3. 对当前仍未解决问题的分级说明，作为后续迭代输入。

对应分支中的核心实现提交包括：

- `e3dae5b Refactor agent prompt assembly and scaffold-based templates`
- `b12d27d feat: implement progressive tool discovery and expansion`
- `022d623 feat: strengthen memory prompt orchestration`

其中，本文档重点覆盖的是 `022d623` 所代表的 memory enhancement，以及它依赖的 prompt builder / scaffold / tool exposure 基础设施。

## 2. 问题定义

OpenFang 在改造前已经存在多种与记忆相关的载体：

1. 共享 KV 长期记忆：`memory_store` / `memory_recall`
2. agent workspace 中的长期约束文件：`MEMORY.md`
3. 工作区沉淀文件：`memory/*.md`
4. 共享 KV 中的 `session_*` 摘要条目

但在真实运行链路里，这些能力没有形成稳定闭环，主要断点有四类：

1. agent workspace 的 `MEMORY.md` 会被 scaffold 生成，但不会稳定进入每轮 system prompt。
2. prompt 对 memory 的使用协议过弱，模型经常跳过 recall/store。
3. 只有精确 key 的 `memory_recall`，模型不知道 key 时只能盲猜。
4. 即使 recall 过，turn 级上下文和历史裁剪也可能让记忆内容在真正请求前失效。

结果是：系统“拥有记忆组件”，但模型并没有稳定获得“看见记忆、发现记忆、引用记忆”的执行路径。

## 3. 本次改造的目标与非目标

### 3.1 目标

本次实现的目标是把长期记忆链路补齐到“在当前架构下可实际工作”的程度：

1. 让 agent workspace 的 `MEMORY.md` 真正进入 prompt 编排链路。
2. 在每轮请求前自动提供少量动态记忆上下文，而不是完全依赖模型临场主动 recall。
3. 提供 `memory_list`，把记忆访问从“猜 key”升级为“先发现、再精确读取”。
4. 把变更控制在 prompt builder、agent loop、tool runner、kernel bridge、兼容层和 scaffold 边界上，不重做底层 memory substrate。
5. 保持 system prompt 稳定，避免因为动态上下文频繁变动而破坏 provider prompt caching。

### 3.2 非目标

本次实现明确没有做以下事情：

1. 不自动把整个 `memory/` 目录塞进 prompt。
2. 不重建底层 memory substrate，也不引入新的独立存储系统。
3. 不在本次实现中引入完整的语义检索、标签索引或混合检索。
4. 不在本次实现中完成全部 prompt architecture 去重与注意力治理。
5. 不在本次实现中定义完整 memory lifecycle，例如 TTL、冲突合并、写入准入和清理策略。

因此，当前实现应被理解为“长期记忆可用性增强”，而不是“完整记忆治理系统”。

## 4. 架构原则

本次改造遵循以下架构原则：

### 4.1 Prompt 组装集中化

静态 prompt 组装集中在 `openfang-runtime::prompt_builder` 中，避免在 kernel、agent loop 和 tool runner 中分散拼接 system prompt。

### 4.2 动静分层

- 静态、相对稳定的内容进入 system prompt。
- 动态、turn-specific 的内容以独立消息注入，不污染 system prompt 稳定性。

这保证了 memory enhancement 不会和 provider cache 策略直接冲突。

### 4.3 通过接口解耦 runtime 与 kernel

- prompt 数据通过 `PromptContext` 传入 runtime。
- memory capability 通过 `KernelHandle` trait 暴露给 runtime。

这样 memory 相关增强不需要让 runtime 直接依赖 kernel 内部实现细节。

### 4.4 边缘适配而非核心重写

兼容层、tool profile、wizard、migrate 只负责暴露新能力和保持旧配置可运行，不把业务规则散落到多个入口。

## 5. 当前实现的架构分层

### 5.1 工作区身份文件层

本分支已经把 agent 的 workspace identity files 规范化为一组稳定入口，由 kernel scaffold 负责首次生成：

- `SOUL.md`
- `USER.md`
- `TOOLS.md`
- `MEMORY.md`
- `AGENTS.md`
- `BOOTSTRAP.md`
- `IDENTITY.md`
- `HEARTBEAT.md`（仅 autonomous agent）

实现位置：

- `crates/openfang-kernel/src/kernel.rs`

关键点：

1. `generate_identity_files()` 使用 `create_new(true)`，默认不覆盖用户已编辑文件。
2. agent workspace 的 `MEMORY.md` 默认模板只提供占位说明，不主动灌入高噪声内容。
3. memory enhancement 复用这个身份文件体系，不单独引入新的“长期记忆文件类型”。

这一层的价值是把长期约束、用户偏好、本地环境说明、长期记忆协议等内容留在 workspace 边界，而不是散落进 manifest 或运行时代码里。

## 6. Prompt 组装层

### 6.1 `PromptContext` 承接静态上下文

当前 system prompt 的输入已经集中到 `PromptContext`：

- base manifest system prompt
- granted tools
- visible skills / MCP summary
- workspace identity files
- canonical context 摘要
- `MEMORY.md`
- channel / date / runtime context

实现位置：

- `crates/openfang-runtime/src/prompt_builder.rs`
- `crates/openfang-kernel/src/kernel.rs`

本次 memory enhancement 的直接结构性改动是：

1. `PromptContext` 新增 `memory_md: Option<String>`。
2. kernel 在两条主要 prompt 构建路径中都通过 `read_identity_file(..., "MEMORY.md")` 注入该字段。
3. runtime 统一通过 `build_system_prompt(&PromptContext)` 输出最终 system prompt。

这样，agent workspace 的 `MEMORY.md` 注入不再依赖某一条特定执行链路，而是成为 prompt builder 的正式输入。

### 6.2 System prompt 的当前分层顺序

当前 `build_system_prompt()` 的顺序大致为：

1. Agent Identity
2. Current Date
3. Tool Use Strategy
4. Immediate Tools
5. Skills
6. Tool Discovery
7. MCP summary
8. Workspace runtime context
9. Channel
10. Safety
11. Operational Guidelines
12. Workspace guidance sections
13. Memory Recall protocol
14. Heartbeat / Bootstrap（按条件）

其中，workspace guidance sections 在 `Full` 模式下包含：

- `AGENTS.md`
- `SOUL.md`
- `TOOLS.md`
- `IDENTITY.md`
- `USER.md`
- `MEMORY.md`

在 `Minimal` 模式下，仅保留：

- `AGENTS.md`
- `SOUL.md`
- `TOOLS.md`
- `MEMORY.md`

这保证 subagent 不会携带过量人物设定和辅助上下文。

### 6.3 `MEMORY.md` 的静态注入

agent workspace 的 `MEMORY.md` 当前作为 `Long-Term Memory` section 进入 system prompt。

实现位置：

- `crates/openfang-runtime/src/prompt_builder.rs`

关键行为：

1. `Full` 模式下 `MEMORY.md` 上限为 2400 chars。
2. `Minimal` 模式下 `MEMORY.md` 上限为 1600 chars。
3. 注入发生在 workspace guidance sections 内，而不是零散拼接到别的位置。

这使得 agent workspace 的 `MEMORY.md` 职责明确为：长期协议、长期偏好、稳定约束、长期项目上下文。

### 6.4 占位文件过滤

为了避免 scaffold 默认模板污染 prompt，`prompt_builder` 对部分 workspace 文件做了占位识别：

- `USER.md`
- `TOOLS.md`
- `MEMORY.md`

其中，只有当 agent workspace 的 `MEMORY.md` 内容不等于默认模板时，它才会被注入。

价值是：

1. 默认 scaffold 能直接生成文件，但不会自动浪费上下文。
2. 用户一旦填入真实内容，就能立即进入 prompt 编排链路。

### 6.5 动态上下文不进入 system prompt

当前分支已经明确把两个高变化度上下文从 system prompt 中拆出：

1. canonical context
2. dynamic memory context

原因是它们会频繁变化，如果放进 system prompt，会显著破坏 provider prompt caching。

因此：

- `build_canonical_context_message()` 生成独立 user message。
- `build_memory_context_message()` 生成独立 user message。

这一步不是附带优化，而是 memory enhancement 能稳定工作的前提之一：

- 静态长期协议留在 system prompt。
- 动态 turn 级记忆留在消息层。

## 7. Agent Loop 层：动态记忆上下文

### 7.1 动态记忆来源

在真正发起 LLM 请求前，agent loop 会组装动态记忆上下文，来源包括：

1. `_memories` 中的 recalled memory fragments
2. 共享 KV 中最近的 `session_*` 摘要

实现位置：

- `crates/openfang-runtime/src/agent_loop.rs`
- `crates/openfang-runtime/src/prompt_builder.rs`

具体函数：

- `format_recalled_memory_fragments()`
- `load_recent_session_summaries()`
- `build_memory_context_message()`

### 7.2 recalled fragments 的格式化策略

`format_recalled_memory_fragments()` 负责对 recall 结果做轻量整理：

1. 去掉空内容。
2. 优先使用 metadata 中的 `key` 作为 label。
3. 如果没有 key，则退化为使用 scope。
4. 对渲染后的片段做去重。
5. 最多保留 5 条。

这样动态记忆上下文不是原始 recall dump，而是更适合直接给模型阅读的短片段列表。

### 7.3 `session_*` 摘要的注入策略

`load_recent_session_summaries()` 从共享 KV 中枚举当前 agent 的 KV：

1. 只保留 key 以 `session_` 开头的条目。
2. 把 value 统一转成文本表示。
3. 进行非空过滤与短截断。
4. 按 key 倒序排序。
5. 默认只取最近 3 条。

这个策略本质上是时间邻近型动态摘要注入，不是语义检索。

### 7.4 独立消息注入路径

agent loop 在构造最终 `messages` 时，会：

1. 从 manifest metadata 中拿 `canonical_context_msg`
2. 构造 `memory_context_msg`
3. 把这两条消息 prepend 到消息列表前部
4. 然后再进入模型调用循环

这一点非常关键：

- agent workspace 的 `MEMORY.md` 是静态长期约束
- `memory_context_msg` 是 turn 级动态历史

两者职责不同，当前实现已经在结构上分离。

## 8. 历史裁剪与上下文保留

### 8.1 发现的问题

在 live 验证中，出现过一个真实回归：

1. dynamic memory context 虽然被 prepend 到消息前面
2. 但长会话下历史裁剪从前向后截断
3. 刚插进去的 context message 反而最先被裁掉

这会让 memory enhancement 看似“已经注入”，但在真正发给模型时失效。

### 8.2 修复方式

为了解决这个问题，agent loop 新增：

- `trim_messages_for_prepended_context(messages, reserved_slots)`

含义是：

1. 先为 prepend 的上下文消息预留槽位
2. 再对普通历史消息做裁剪
3. 保证 canonical context 和 memory context 不会被本轮自己的裁剪逻辑先吞掉

当前非 streaming 和 streaming 两条主路径都已接入该修复。

## 9. Memory Tool 层：从精确 recall 到发现再 recall

### 9.1 新能力：`memory_list`

本次新增的核心 memory tool 是 `memory_list`。

用途：

1. 列出共享 memory 中已有的 key
2. 可按 prefix 过滤
3. 可限制返回条数
4. 可选择是否返回 value
5. 支持把 `query` 作为 `prefix` 的兼容输入

实现位置：

- `crates/openfang-runtime/src/tool_runner.rs`
- `crates/openfang-runtime/src/kernel_handle.rs`
- `crates/openfang-kernel/src/kernel.rs`

### 9.2 当前实现形态

当前 `memory_list` 的行为是典型 KV list：

1. kernel 通过 `memory.list_kv(agent_id)` 取回当前共享 memory agent 下的全部 KV。
2. 可选地按 `starts_with(prefix)` 过滤。
3. 按 key 升序排序。
4. 可选地按 `limit` 截断。
5. tool runner 将结果序列化为 JSON 数组返回给模型。

这意味着当前 `memory_list` 是“key discovery tool”，不是“语义搜索引擎”。

### 9.3 Prompt 协议同步

新增 tool 后，prompt 中的 memory guidance 也同步更新：

- key 明确时优先 `memory_recall`
- key 不明确时先 `memory_list`
- 长期稳定信息需要时使用 `memory_store`

这使 memory protocol 从“可能有 recall 能力”变成了明确的两段式调用协议。

## 10. Kernel Bridge 与共享存储边界

### 10.1 `KernelHandle` 扩展

为了让 runtime 侧能够调用新的 memory discovery 能力，`KernelHandle` trait 增加了：

- `memory_list(prefix, limit) -> Vec<(String, Value)>`

这保持了 runtime 与 kernel 的依赖方向不变：

- runtime 只依赖 trait
- kernel 提供具体实现

### 10.2 当前共享 memory 的边界

当前 `memory_store` / `memory_recall` / `memory_list` 都通过 `shared_memory_agent_id()` 落到同一个共享 memory agent 空间上。

这与本次目标一致：

1. 不重做存储层。
2. 先在既有共享 KV substrate 上补齐发现与注入链路。

但这也意味着，后续若要继续演进 memory governance，需要在此边界之上继续加 namespace、隔离和生命周期规则，而不是继续把规则散到 runtime prompt 层。

## 11. 兼容层与暴露面修正

### 11.1 工具兼容映射

为了兼容旧配置和迁移输入，`tool_compat` 中加入了：

- `memory_search -> memory_list`

这保证老数据不会因为新能力命名调整而直接失效。

### 11.2 Agent 模式与 ToolProfile 暴露面

`memory_list` 已进入以下暴露面：

1. `AgentMode::Assist` 的只读工具集合
2. `ToolProfile::Messaging`
3. `ToolProfile::Automation`

这保证 memory discovery 在受限模式下也可用，而不仅限于 Full agent。

### 11.3 Wizard / Scaffold 更新

setup wizard 已同步更新 memory 提示：

1. 当 agent 具有 memory capability 时，会授予 `memory_store` / `memory_recall` / `memory_list`
2. tool hint 明确说明“精确访问用 recall，不知道 key 时先 list”

这样 memory 协议不会只存在于 runtime，而是从 agent 创建入口就开始显式传达。

### 11.4 迁移测试更新

OpenClaw 迁移测试已更新为断言：

- `memory_search -> memory_list`

这一步虽然小，但很重要：它保证了 memory enhancement 不是“新 agent 有、旧 agent 坏掉”的单向增强。

## 12. 当前端到端执行链路

当前记忆链路的端到端行为可概括为：

1. kernel 为 agent workspace 生成并保留 identity files。
2. kernel 构建 `PromptContext`，把 agent workspace 的 `MEMORY.md` 与其他 workspace files 读入。
3. `prompt_builder` 生成稳定的 system prompt，并在其中静态注入 agent workspace 的 `MEMORY.md`。
4. canonical context 被单独生成成 user message，避免 system prompt 频繁变化。
5. agent loop 在每轮请求前进行 memory recall，并收集最近的 `session_*` 摘要。
6. recalled fragments 和 recent session summaries 被组装为 `memory_context_msg`。
7. `canonical_context_msg` 与 `memory_context_msg` 被 prepend 到本轮消息列表前部。
8. 若历史过长，先保留 prepended context 再裁剪普通历史。
9. 模型在工具侧可用 `memory_list -> memory_recall -> memory_store` 形成完整闭环。

## 13. 当前系统行为与使用边界

改造完成后，当前系统的行为边界如下：

1. 每轮请求都会尝试将 agent workspace 的 `MEMORY.md` 注入 system prompt。
2. 默认占位版的 agent workspace `MEMORY.md` 不会注入。
3. 每轮请求前都会尝试注入少量动态记忆上下文。
4. 动态记忆上下文目前由 recalled fragments 与 recent `session_*` 摘要组成。
5. 模型在 key 不明确时可以先使用 `memory_list` 再 `memory_recall`。
6. `memory_list` 当前是 prefix/key discovery，不是语义搜索。
7. `memory/` 目录不会自动进入 prompt。
8. 用户若询问“昨天/上周某天具体聊了什么”，仍应通过 `file_read` / `file_search` 检查工作区 `memory/` 下的文件。

## 14. `MEMORY.md` 的当前职责定义

在当前实现里，`MEMORY.md` 适合承载：

1. 长期稳定的行为协议
2. 记忆工具使用规则
3. 稳定的用户偏好
4. 项目长期约束、架构边界和固定协作约定

不适合放进去的内容：

1. 高频变化的短期事实
2. 某一次对话的临时结论
3. 具体日期定位型历史事件
4. 会快速过期的操作状态

这些更适合沉淀到：

- KV memory
- `session_*` 摘要
- `memory/*.md`

## 15. Prompt Architecture 相关但未纳入本次实现的内容

本分支中的 memory enhancement 已经和当前 prompt architecture 发生结构性衔接，但仍有一部分相邻问题没有在本次实现中解决，应该明确标为后续工作，而不是混入“当前设计已完成”：

1. `AGENTS.md` / `USER.md` / `TOOLS.md` / `MEMORY.md` 的职责进一步去重与模板优化。
2. system prompt 各板块的注意力预算、token 预算和优先级治理。
3. 跨 section 冲突仲裁，例如 `USER.md`、`MEMORY.md`、KV recall 之间谁更权威。
4. assistant 专属 `MEMORY.md` 的生成质量、维护策略和自动收敛机制。

也就是说：

- 当前实现已经解决了 memory visibility、memory discovery 和 turn-level injection。
- 当前实现尚未完成完整的 prompt attention architecture。

## 16. 风险与后续迭代优先级

下面的优先级不是“当前代码出错”，而是“当前设计下一步最应该补齐的缺口”。

### P1：应优先补齐

#### P1.1 Memory lifecycle / governance 缺失

当前设计没有定义：

- 什么内容允许 `memory_store`
- 如何去重
- 如何处理冲突写入
- 如何淘汰过期 memory
- 哪些信息应晋升到 `MEMORY.md`

如果不补，长期会把共享 KV 池变成高噪声区，直接反噬 `memory_list` 与 recall 质量。

#### P1.2 Retrieval quality 仍偏弱

当前 `memory_list` 只是基于 key/prefix 的发现能力，不是 query-aware retrieval。

这意味着：

- key 命名不规范时，模型仍然可能列出很多结果却找不到真正相关项。
- “很久以前但高度相关”的信息，仍可能因为不在 recent `session_*` 中而无法被自动带出。

下一阶段应优先考虑 namespace、tag、schema，随后再看是否引入混合检索。

#### P1.3 共享 memory 的隔离与权限边界

当前 `memory_list` 暴露的是共享 memory agent 下的 KV 视图。后续如果系统继续扩张到多 workspace、多 agent、多用户场景，需要进一步定义 namespace 和访问边界，避免 memory discovery 过宽。

#### P1.4 可观测性不足

当前验证证明 wiring 生效，但没有形成完整质量指标，例如：

- recall 命中率
- `memory_list -> memory_recall` 转化率
- prompt token 增量
- 延迟影响
- 用户纠错率
- memory 污染率

没有这些指标，后续很难区分“记忆真的有帮助”还是“只是多塞了一些上下文”。

### P2：第二阶段优化

#### P2.1 Prompt 文档职责重整

`MEMORY.md` 之外，`AGENTS.md`、`USER.md`、`TOOLS.md`、`BOOTSTRAP.md`、`SOUL.md`、`IDENTITY.md` 之间仍然存在一定的职责重叠。

后续应进一步明确：

- 哪些规则属于 system prompt 固定指令
- 哪些属于 workspace guidance
- 哪些属于长期记忆
- 哪些属于一次性 bootstrap ritual

#### P2.2 注意力预算与冲突治理

当前 system prompt 已有 section ordering 和字符上限，但还没有做到：

- token-aware budget
- 跨 section 去重
- 任务类型驱动的重排
- 明确的冲突优先级

这属于 prompt architecture 的下一阶段，而不是本次 memory enhancement 的直接目标。

#### P2.3 Assistant 专属 `MEMORY.md` 模板与维护策略

当前实现解决了 `MEMORY.md` 能被消费，但没有完整解决 assistant 的 `MEMORY.md` 如何高质量生成、如何更新、如何收敛。

### P3：低优先级改进

#### P3.1 占位文件识别的鲁棒性

当前 `MEMORY.md` 占位识别仍然是模板匹配型逻辑。它足以过滤默认 scaffold，但不是完整的结构化有效性判断。后续可考虑更显式的 schema 或元数据约束。

## 17. 验证状态

本次实现已经完成以下验证：

- `cargo build --workspace --lib`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`

并做了 live integration 验证：

1. daemon 可启动
2. `/api/health` 与 `/api/agents` 可返回
3. `/api/agents/{id}/message` 可进入 runtime 执行链路
4. 通过 `llm.log` 可确认 `MEMORY.md` 与 `memory_list` 已进入实际 prompt / tool 路径
5. live 验证中发现并修复了 prepend memory context 被历史裁剪吞掉的问题

验证时外部 provider completion 仍受地域或鉴权约束，未形成完整成功回答；但这不影响本次对 prompt wiring、tool exposure、消息拼装与裁剪保留逻辑的验证。

## 18. 涉及文件

本次 memory enhancement 及其直接接线涉及以下文件：

- `crates/openfang-runtime/src/prompt_builder.rs`
- `crates/openfang-runtime/src/agent_loop.rs`
- `crates/openfang-runtime/src/tool_runner.rs`
- `crates/openfang-runtime/src/kernel_handle.rs`
- `crates/openfang-kernel/src/kernel.rs`
- `crates/openfang-kernel/src/wizard.rs`
- `crates/openfang-types/src/agent.rs`
- `crates/openfang-types/src/tool_compat.rs`
- `crates/openfang-migrate/src/openclaw.rs`
- `docs/agent_memory_enhancement_plan.md`

其中：

- `prompt_builder.rs` 与 `agent_loop.rs` 构成记忆注入主链路。
- `tool_runner.rs`、`kernel_handle.rs`、`kernel.rs` 构成 `memory_list` 的能力闭环。
- `wizard.rs`、`agent.rs`、`tool_compat.rs`、`openclaw.rs` 负责暴露面和兼容性。

## 19. 结论

当前分支已经把长期记忆从“存在一些零散能力”提升为“有稳定执行链路的系统能力”，核心落地点包括：

1. agent workspace 的 `MEMORY.md` 正式进入集中式 prompt builder。
2. 占位的 agent workspace `MEMORY.md` 不再污染 prompt。
3. dynamic memory context 在 agent loop 中以独立消息形式注入。
4. `memory_list` 补齐了发现 key 的能力。
5. `memory_search -> memory_list` 的兼容映射、tool profile 暴露和 wizard 提示已同步完成。
6. 历史裁剪已修复对 prepended memory context 的破坏。

因此，当前实现已经完成“长期记忆可用性增强”的主体目标。

下一阶段不应继续把所有问题都堆进 memory 文档，而应沿两个相对解耦的方向推进：

1. memory governance / retrieval quality
2. prompt architecture / attention governance

两者相关，但不应在同一轮改造里混成一个高耦合大改。
