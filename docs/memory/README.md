# 记忆协作文档约定

`docs/memory/` 是持续推进记忆计划的唯一管理目录。

## 固定入口

用户后续只需要发出一句指令：

- `阅读 docs/memory/execution_state.md，继续工作`

当收到这句指令时，助手必须：

1. 先读取 `docs/memory/execution_state.md`
2. 再读取其中 `## 设计文档` 一节列出的所有设计文档
3. 检查当前分支和 worktree 状态
4. 只在 `## 当前阶段` 的范围内继续工作
5. 在结束一轮实质性工作前更新 `docs/memory/execution_state.md`

## 结构冻结

为了保证跨电脑切换时的连续性，未经用户明确批准，助手不得重命名、移动、删除或改动以下文件的章节结构：

- `docs/memory/README.md`
- `docs/memory/execution_state.md`
- `docs/memory/agent_memory_enhancement_plan.md`

未经批准允许做的变更：

- 更新 `docs/memory/execution_state.md` 中各字段的内容
- 仅在当前阶段确有需要时，在 `docs/memory/` 下新增设计文档
- 更新 `docs/memory/execution_state.md` 中的引用路径

未经批准禁止做的变更：

- 用其他文件替换当前入口文件
- 修改 `docs/memory/execution_state.md` 的章节标题
- 把记忆计划管理文档移回 `docs/`
- 再创建第二套并行的执行状态文件

## 文件职责

- `docs/memory/execution_state.md`：执行状态、分支和 worktree 连续性、下一步动作
- `docs/memory/agent_memory_enhancement_plan.md`：方案设计与阶段性架构说明
