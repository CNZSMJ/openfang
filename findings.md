# Findings & Decisions

## Requirements
- Read `docs/skill-progressive-loading-design.md` and implement the development plan.
- Keep Phase 1 scoped to skills only.
- Preserve the existing skill execution path and `skill_get_instructions`.
- Remove default prompt injection of the full skill catalog.
- Add runtime discovery for local skills.
- Make API tool listing more consistent with runtime-visible skills.
- Follow repository verification requirements, including live integration checks after wiring changes.

## Research Findings
- `crates/openfang-runtime/src/prompt_builder.rs` currently injects a `## Skills` section listing every skill in `PromptContext.skills`.
- `crates/openfang-runtime/src/tool_runner.rs` already exposes `skill_get_instructions`, but there is no `skill_search` tool yet.
- `crates/openfang-kernel/src/kernel.rs` already merges builtin tools, skill-provided executable tools, and MCP tools in `available_tools()`.
- `crates/openfang-api/src/routes.rs` documents `/api/tools` as built-in + MCP only, while `/mcp` already aggregates builtin + skills + MCP.
- The design doc explicitly says Phase 1 should keep external names and start with `skill_search(query, top_k)` plus `skill_get_instructions(skill_name)`.
- After implementation review, the current runtime behavior is now clear:
  - The LLM directly receives `Vec<ToolDefinition>` through `CompletionRequest.tools`
  - builtin, skill, and MCP all become `ToolDefinition` before the provider call
  - prompt text only lists granted tool names and hints, but provider tool APIs receive the full definitions
  - `agent_loop` currently holds `available_tools` as a fixed slice for the whole loop iteration sequence
  - current `skill_search` results do not auto-expand any new `ToolDefinition` into the next LLM request

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Add local skill discovery via a first-class tool instead of overloading prompt metadata | This matches the intended `discover -> reference -> expand` flow |
| Keep some lightweight prompt guidance about discovery | Removing the catalog entirely still needs a hint that discovery exists |
| Reuse registry-backed skill metadata for both runtime and API views | Prevents divergence between what the model can discover and what APIs expose |
| Move visibility filtering for discovery into kernel-backed `skill_search` | Runtime search must respect each agent's skill allowlist |
| Add `skill_search` and `skill_get_instructions` to standard tool profiles | The new prompt protocol would be dead code for non-Full profiles otherwise |
| Treat Anthropic `tool_search` as the target model for the next stage | This is the cleanest way to unify builtin, skill, and MCP deferred loading |
| Keep external `tool_reference` thin and Anthropic-compatible | The next-stage complexity belongs in automatic expansion, not in a rich search-result object |
| Do not prematurely introduce `ToolCatalogEntry + metadata` as a required abstraction | The current minimum useful internal model can remain a `ToolCatalog[ToolDefinition]` plus dynamic expansion logic |
| Focus next-stage design on dynamic tool expansion in `agent_loop` | That is the concrete gap preventing `tool_search -> expanded tools -> tool_use` today |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| API surface appears inconsistent between `/api/tools` and `/mcp` | Will address by extending `/api/tools` to include skill resources |
| `tool_runner` outer function returns `ToolResult`, so search errors cannot use `?` | Switched the new `skill_search` branch to return explicit `ToolResult` errors |
| Early discussion overfit on richer `tool_reference` payloads | Re-read Anthropic docs and reconstructed the actual OpenFang loop; determined the real missing piece is host-side automatic expansion |
| Builtin / skill / MCP authorization semantics are not unified today | Recorded as an explicit next-stage constraint: authorization must be computed per current source-specific rules before deferred search runs |

## Resources
- `docs/skill-progressive-loading-design.md`
- `/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-runtime/src/prompt_builder.rs`
- `/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-runtime/src/tool_runner.rs`
- `/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-kernel/src/kernel.rs`
- `/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-api/src/routes.rs`
- `/Users/jiahaohuang/workspace_ai/openfang-0.1.0/refactor-tool-progressive-design/crates/openfang-runtime/src/agent_loop.rs`
- [Anthropic Tool search tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool)

## Visual/Browser Findings
- No browser or image findings in this task.
