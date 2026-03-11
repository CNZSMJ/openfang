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
  - `agent_loop` used to hold `available_tools` as a fixed slice for the whole loop iteration sequence
  - current `ToolRunner` now auto-expands matching hidden deferred tools after `tool_search`, and that live path has been proven for both skill and MCP sources
  - a cleaner next step is to move loop-local tool-surface ownership into a stateful runtime object instead of teaching `agent_loop` about hidden/deferred sets directly
  - skill-provided callable tools can be recognized at runtime by asking `SkillRegistry` which skill provides a given tool name
  - `ToolDefinition` now has an explicit `defer_loading` flag, so deferred exposure is no longer only an implicit runtime convention
  - `tool_search` is now the only public discovery entry for deferred tools
  - `tool_search` now searches `ToolRunner`'s generic hidden-tool map directly and returns tool-centric results (`name`, `description`, `source`, `provider`) instead of reusing skill-search payloads
  - live validation confirms the `ToolRunner`-owned flow works within one top-level message:
    - `tool_search`
    - host-side expansion of matching hidden skill tools
    - same-turn invocation of the newly visible tool
  - live validation also confirms the public `tool_search` alias now exercises that same path end to end on a fresh MiniMax-backed probe agent
  - live validation also confirms the generic `tool_search` path can discover, expand, and invoke a real deferred MCP tool (`mcp_minimax_web_search`) in one top-level message
  - live validation also confirms the expanded tool surface is loop-scoped rather than session-scoped; a new top-level message starts from the initial visible set again
  - live validation should now target `tool_search`; a fresh probe agent is still useful when the default legacy `assistant` surface does not match the rollout
  - builtin tools should remain `defer_loading = false`; the deferred rollout currently applies only to skill- and MCP-provided tools
  - external `/mcp` clients should not see `tool_search` or `skill_get_instructions`, because that transport bypasses the loop-local `ToolRunner` state those tools depend on
  - `skill_search` no longer belongs in the system at all; `tool_search` is now the only discovery path
  - `skill_get_instructions` still remains in the default surface, but `tool_search` can now return `skill_manual` results so prompt-only skill workflows also go through the unified discovery path first

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
| Introduce a stateful `ToolRunner` as the runtime owner of the loop-local visible tool surface | This keeps `agent_loop` clean and avoids smuggling `tool_search`-specific state through a fake generic result type |
| Use the existing `skill_search` protocol as the first automatic-expansion trigger | This preserves the working Phase 1 API while validating the host-side expansion mechanism |
| Keep dynamic expansion scoped to a single top-level agent loop | This matches the current `ToolRunner` lifetime and avoids mutating persistent agent manifests or session-global tool state |
| Add `defer_loading` to `ToolDefinition` before introducing public `tool_search` | The runtime needed an explicit deferred-exposure flag before the external discovery API could be renamed |
| Introduce `tool_search` as the primary discovery name before removing `skill_search` | This lets prompts, tool profiles, and agents migrate without breaking the already validated skill-only flow |
| Keep `skill_search` skill-scoped even after `tool_search` becomes generic | This preserves compatibility while letting the new public path evolve toward cross-source deferred discovery |
| Keep `skill_search` as a compatibility entry point until `tool_search` covers all deferred sources | Removing it earlier would break the currently validated skill-only discovery flow without a true unified replacement |
| Validate the public `tool_search` alias with a real provider before removing compatibility language | Prompt/profile migration is only credible once the renamed entry is proven live, not just in unit tests |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| API surface appears inconsistent between `/api/tools` and `/mcp` | Will address by extending `/api/tools` to include skill resources |
| `tool_runner` outer function returns `ToolResult`, so search errors cannot use `?` | Switched the new `skill_search` branch to return explicit `ToolResult` errors |
| Early discussion overfit on richer `tool_reference` payloads | Re-read Anthropic docs and reconstructed the actual OpenFang loop; determined the real missing piece is host-side automatic expansion |
| Builtin / skill / MCP authorization semantics are not unified today | Recorded as an explicit next-stage constraint: authorization must be computed per current source-specific rules before deferred search runs |
| Renaming `available_tools` directly at the loop boundary touches both streaming and non-streaming paths | Started by renaming the kernel-side concept to `authorized_tools` and routing both loops through a shared stateful `ToolRunner` |
| Jumping straight to public `tool_search` would mix protocol migration with runtime validation | Used `skill_search` as the transition point so dynamic expansion can be validated independently of public naming changes |
| Live validation with the default `assistant` initially looked broken because the model claimed it lacked `skill_search` | Checked the agent manifest and confirmed the legacy `assistant` capability list does not include `skill_search`, so the runtime behavior was correct for that agent |
| A naive `defer_loading` rollout could hide non-skill tools without any expansion path | Landed MCP as the first real non-skill deferred source under `tool_search`, and validated the same-message expansion path live before considering broader builtin rollout |
| Builtin visibility should not be mixed into the deferred rollout | Keep builtin tools at `defer_loading = false`; the current deferred/discovery model is only for skill- and MCP-provided tools |
| Discovery helpers should not be exported over stateless MCP transports | `/mcp` bypasses the loop-local `ToolRunner`, so it should expose executable tools only and filter out `tool_search`, `skill_search`, and `skill_get_instructions` |
| Remove `skill_search` from the default surface before deleting it outright | This migrates prompts and profiles onto `tool_search` first, while preserving the builtin alias for older agents and manifests |
| The first live `tool_search` alias probe did not perform an external GitHub write even though the tool was invoked | Confirmed the discovered `github_comment` skill is prompt-only, so the host correctly returned its instructional tool result instead of pretending a real side effect happened |
| The first post-refactor live probe still returned old skill-search payloads from `tool_search` | Confirmed the daemon was running a stale `target/debug/openfang`; rebuilt `openfang-cli`, restarted the daemon, and re-ran the probe against the updated binary |

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
