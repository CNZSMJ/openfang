# Progress Log

## Session: 2026-03-11

### Phase 1: Requirements & Discovery
- **Status:** complete
- **Started:** 2026-03-11
- Actions taken:
  - Read `using-superpowers` and `planning-with-files` skill instructions.
  - Read `docs/skill-progressive-loading-design.md`.
  - Located current skill prompt injection, runtime tool execution, kernel tool assembly, and API tool listing code paths.
  - Captured initial requirements and technical direction in planning files.
- Files created/modified:
  - `task_plan.md` (created)
  - `findings.md` (created)
  - `progress.md` (created)

### Phase 2: Planning & Structure
- **Status:** complete
- Actions taken:
  - Chose a Phase 1 implementation that keeps `skill_search -> skill_get_instructions` as the public protocol.
  - Moved prompt behavior from enumerated skill summaries to a static discovery protocol.
  - Decided to enforce agent-visible skill filtering in kernel-backed search instead of runtime-only registry access.
- Files created/modified:
  - `task_plan.md`
  - `findings.md`
  - `progress.md`

### Phase 3: Implementation
- **Status:** complete
- Actions taken:
  - Added lexical local-skill search to `openfang-skills::registry`.
  - Added the builtin `skill_search` tool and kept `skill_get_instructions`.
  - Updated `PromptContext` and `prompt_builder` so the system prompt advertises the discovery protocol instead of listing every skill.
  - Updated kernel prompt assembly and `KernelHandle` to support visibility-aware skill search.
  - Extended standard tool profiles so discovery tools are available outside `Full`.
  - Extended `/api/tools` to include skill-provided callable tools.
  - Added targeted registry and prompt/tool tests.
- Files created/modified:
  - `crates/openfang-skills/src/lib.rs`
  - `crates/openfang-skills/src/registry.rs`
  - `crates/openfang-runtime/src/prompt_builder.rs`
  - `crates/openfang-runtime/src/tool_runner.rs`
  - `crates/openfang-runtime/src/kernel_handle.rs`
  - `crates/openfang-kernel/src/kernel.rs`
  - `crates/openfang-types/src/agent.rs`
  - `crates/openfang-api/src/routes.rs`

### Phase 4: Testing & Verification
- **Status:** complete
- Actions taken:
  - Ran targeted tests for skill registry search.
  - Ran targeted tests for prompt builder skill protocol and builtin tool definitions.
  - Fixed a compile error in the new `skill_search` tool branch.
  - Ran `cargo build --workspace --lib`.
  - Ran `cargo test --workspace` and fixed follow-on failures in `openfang-migrate` and prompt formatting expectations.
  - Ran `cargo clippy --workspace --all-targets -- -D warnings`.
  - Built the debug CLI binary and started a live daemon with `target/debug/openfang start`.
  - Verified `/api/health`, `/api/tools`, `/api/agents`, and dashboard HTML against the configured local API port `4200`.
  - Confirmed `/api/tools` exposes builtin discovery tools on the live daemon.
  - Re-ran a real MiniMax-backed live message against `assistant`; the call succeeded and budget usage increased.
  - Confirmed the default legacy `assistant` manifest still does not authorize `skill_search`, so that specific agent cannot exercise the discovery flow.
  - Created a temporary live validation skill plus fresh probe agents to verify the new runtime behavior end to end.
  - Confirmed the canonical single-message flow works live:
    - `skill_search`
    - automatic expansion of matching hidden callable skill tools
    - same-turn tool invocation of the newly visible skill tool
- Files created/modified:
  - `progress.md`

### Phase 5: Delivery
- **Status:** complete
- Actions taken:
  - Reviewed the final modified files and preserved unrelated user changes.
  - Cleaned up the live daemon process after API validation.
  - Recorded the remaining live-test blocker for real LLM execution.
  - Verified on a fresh MiniMax-backed agent that the intended `skill_search -> skill_get_instructions -> execute` loop works without XML text-tool-call recovery.
  - Narrowed the final change set to the Phase 1 design itself, removing compatibility logic that was not required by the clean-agent validation.
- Files created/modified:
  - `task_plan.md`
  - `progress.md`

### Phase 6: Post-Implementation Design Review
- **Status:** in progress
- Actions taken:
  - Re-read the implemented Phase 1 flow against Anthropic `tool_search` design.
  - Confirmed that OpenFang already unifies builtin, skill, and MCP at the LLM tool-protocol layer: all three become `ToolDefinition` before the LLM call.
  - Confirmed that the current system does **not** yet unify authorization semantics:
    - builtin tools rely more directly on `Capability::ToolInvoke`
    - skill tools rely on `skill_allowlist + model_invocable + host_tools`
    - MCP tools rely on `mcp_servers` allowlist
  - Confirmed that the current `agent_loop` receives `available_tools` as a fixed slice and reuses that same set across iterations.
  - Confirmed that current `skill_search` results do **not** auto-expand new `ToolDefinition` objects into the next LLM request.
  - Narrowed the next-stage target flow to:
    - `llm -> tool_search -> host automatic expansion -> llm tool_use -> ToolCall`
  - Rejected several over-designed directions during review:
    - XML text-tool-call recovery as a requirement of this design
    - legacy full-mode backfill
    - `llm.log` timing changes
    - introducing a heavy `ToolCatalogEntry + metadata` model before proving it is needed
  - Recorded a more minimal next-stage direction:
    - extend `ToolDefinition` with `defer_loading`
    - unify internal runtime search over a `ToolCatalog[ToolDefinition]`
    - keep `tool_reference` thin and Anthropic-compatible
    - solve dynamic tool expansion in `agent_loop`
  - Refined the implementation direction so `available_tools` becomes `authorized_tools`, and a stateful runtime `ToolRunner` owns the current visible tool surface for each agent loop.
  - Started the runtime refactor:
    - added a stateful `ToolRunner` wrapper in `openfang-runtime`
    - updated both non-streaming and streaming agent loops to source request tools and execution allowlists from `ToolRunner`
    - renamed kernel-side assembly semantics from `available_tools` to `authorized_tools`
  - Extended the runtime refactor into a working transition flow:
    - initial visible tools now hide skill-provided callable tools
    - `ToolRunner` intercepts `skill_search` and automatically expands callable tools from matching skills into later turns
    - prompt guidance now explains that matching skill tools may appear after `skill_search`
  - Preserved the external Phase 1 protocol while proving the automatic-expansion mechanism on the existing `skill_search` path.
  - Formalized deferred exposure in the type system:
    - added `ToolDefinition.defer_loading`
    - kernel now marks skill-provided callable tools with `defer_loading = true`
    - `ToolRunner` now derives initial visibility from `defer_loading` instead of an implicit skill-only visibility rule
  - Re-validated the runtime behavior live after the `defer_loading` migration:
    - same-message `skill_search -> auto-expand -> github_comment`
    - actual tool invocation succeeded and returned the expected prompt-only skill result
  - Promoted `tool_search` into the public discovery surface while keeping `skill_search` as a compatibility alias:
    - builtin tool registry now exposes both names
    - standard tool profiles now grant `tool_search`
    - prompt guidance now points to `tool_search` as the primary discovery entry
    - initial live rollout covered skill-backed deferred tools first
  - Reworked the runtime discovery split further:
    - `tool_search` now searches `ToolRunner`'s generic deferred-tool map instead of proxying to kernel skill search
    - `skill_search` remains skill-scoped and keeps the old skill-result payload
    - the generic `tool_search` result is now tool-centric: `name`, `description`, `source`, `provider`
  - Extended the deferred rollout to a real non-skill source:
    - kernel now marks MCP tools with `defer_loading = true`
    - `tool_search` can discover and expand deferred MCP tools in the same top-level message
  - Re-validated the renamed public discovery flow live:
    - same-message `tool_search -> auto-expand -> github_comment`
    - budget metering recorded the probe agent spend
    - the discovered prompt-only skill tool executed, but correctly returned its prompt-context note instead of a side-effecting GitHub write
    - session logs confirmed `tool_search` itself now returned generic tool results rather than legacy skill-search payloads
  - Re-validated the first real non-skill deferred flow live:
    - same-message `tool_search -> auto-expand -> mcp_minimax_web_search`
    - session logs confirmed the generic `tool_search` result returned `{name:"mcp_minimax_web_search", source:"mcp", provider:"minimax"}`
    - the expanded MCP tool executed successfully and `/api/budget/agents/{id}` recorded `live-mcp-tool-searcher` spend
  - Tightened the external MCP surface after the runtime refactor:
    - builtin tools remain `defer_loading = false`
    - `/mcp` now filters out `tool_search`, `skill_search`, and `skill_get_instructions`
    - live `tools/list` confirmed normal tools still appear while loop-only discovery helpers do not
  - Reduced compatibility-surface visibility without removing the old tools yet:
    - standard `ToolProfile`s now grant `tool_search` and `skill_get_instructions`, but no longer grant `skill_search`
    - prompt guidance now speaks in terms of `tool_search` as the default discovery action
    - `skill_search` remains present in the builtin catalog only as a compatibility alias
- Files created/modified:
  - `docs/skill-progressive-loading-design.md`
  - `progress.md`
  - `task_plan.md`
  - `findings.md`
  - `crates/openfang-runtime/src/tool_runner.rs`
  - `crates/openfang-runtime/src/agent_loop.rs`
  - `crates/openfang-kernel/src/kernel.rs`
  - `crates/openfang-runtime/src/prompt_builder.rs`
## Test Results
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Registry search | `cargo test -q -p openfang-skills registry::tests::test_search_matches_name_and_tags -- --exact` | New lexical search test passes | Passed | ✓ |
| Prompt protocol | `cargo test -q -p openfang-runtime prompt_builder::tests::test_skills_section_present -- --exact` | Prompt advertises protocol instead of catalog | Passed | ✓ |
| Builtin tool list | `cargo test -q -p openfang-runtime tool_runner::tests::test_builtin_tool_definitions -- --exact` | Builtin tool registry includes `skill_search` and `tool_search` | Passed | ✓ |
| Workspace build | `cargo build --workspace --lib` | Workspace libraries compile | Passed | ✓ |
| Workspace tests | `cargo test --workspace` | Full test suite passes | Passed | ✓ |
| Workspace clippy | `cargo clippy --workspace --all-targets -- -D warnings` | Zero warnings | Failed only on pre-existing `openfang-cli/src/main.rs` `collapsible_else_if` warnings | △ |
| Live health check | `curl -s http://127.0.0.1:4200/api/health` | API returns healthy status | `{"status":"ok","version":"0.3.34"}` | ✓ |
| Live tools check | `curl -s http://127.0.0.1:4200/api/tools` | Discovery tools are visible on the daemon | Included builtin `skill_search`, `tool_search`, and `skill_get_instructions` | ✓ |
| Live agents check | `curl -s http://127.0.0.1:4200/api/agents` | API responds with agent list | Returned configured MiniMax-backed local agents | ✓ |
| Live dashboard check | `curl -s http://127.0.0.1:4200/ | head` | Dashboard HTML is served | Returned `OpenFang Dashboard` HTML | ✓ |
| Live assistant LLM check | `POST /api/agents/{assistant}/message` | Real provider call succeeds and usage updates | Response returned, `/api/budget` spend increased | ✓ |
| Live dynamic-expansion discovery check | Fresh `live-skill-searcher` agent on `127.0.0.1:4200` | Model should discover a hidden skill tool in one request | Response identified newly visible `github_comment` after `skill_search` | ✓ |
| Live dynamic-expansion single-message execution check | Fresh `live-skill-searcher-2` agent on `127.0.0.1:4200` | Model should search, expand, and invoke the new skill tool in one top-level message | Response quoted `github_comment` tool output after same-turn invocation | ✓ |
| Live `defer_loading` execution re-check | Fresh `live-skill-searcher-defer` agent on `127.0.0.1:4200` | Explicit `defer_loading` wiring should preserve same-message expansion and invocation | Response returned successful `github_comment` output after `skill_search` | ✓ |
| Live `tool_search` alias execution check | Fresh `live-tool-searcher` agent on `127.0.0.1:4200` | Model should use `tool_search`, expand the hidden skill tool, and invoke it in one top-level message | Response quoted the exact prompt-only `github_comment` tool result; `/api/budget/agents` recorded `live-tool-searcher` spend | ✓ |
| Live generic `tool_search` result-shape check | Fresh `live-tool-searcher-generic` agent on `127.0.0.1:4200` after rebuilding `openfang-cli` | `tool_search` should return generic tool-centric results and still expand/invoke the hidden skill tool | Session log showed `tool_search` returning `{name:\"github_comment\", source:\"skill\", provider:\"live-tool-search-helper\"}` before same-message `github_comment` invocation; `/api/budget/agents` recorded spend | ✓ |
| Live MCP deferred expansion check | Fresh `live-mcp-tool-searcher` agent on `127.0.0.1:4200` with `mcp_servers = [\"MiniMax\"]` | `tool_search` should discover a deferred MCP tool, expand it, and invoke it in the same top-level message | Session log showed `tool_search` returning `mcp_minimax_web_search` with `source:\"mcp\"`, then same-message `mcp_minimax_web_search` returned live web search results; `/api/budget/agents/{id}` recorded `0.0440814` USD spend | ✓ |
| Live MCP HTTP tools-list filter check | `POST /mcp` `tools/list` on `127.0.0.1:4200` | External MCP surface should still include normal builtin/MCP tools but exclude loop-only discovery helpers | `file_read` and `mcp_minimax_web_search` remained visible while `tool_search`, `skill_search`, and `skill_get_instructions` were absent | ✓ |
| Live API tools compatibility check | `GET /api/tools` on `127.0.0.1:4200` | Internal API catalog should still expose compatibility tools during migration | `tool_search`, `skill_search`, and `skill_get_instructions` all remained present in `/api/tools` | ✓ |

## Error Log
| Timestamp | Error | Attempt | Resolution |
| 2026-03-11 | `openfang-migrate` profile-count test failed after tool-profile expansion | Ran full workspace tests | Updated test to assert `skill_search` and `skill_get_instructions` presence |
| 2026-03-11 | Prompt builder ordering/soul tests failed after static-skill rewrite | Ran full workspace tests | Restored a stable `## Tone` section format in `build_soul_section()` |
| 2026-03-11 | Real live LLM message test could not run | Checked environment and daemon auth state | Blocked because `GROQ_API_KEY` is unset locally |
| 2026-03-11 | Initial MiniMax failure sample suggested XML-style text tool calls might require recovery | Re-ran the same scenario on a fresh agent after isolating variables | Determined XML recovery was not required for the Phase 1 flow and removed it |
| 2026-03-11 | Existing legacy `assistant` agent exposed `## Skills` in prompt while missing discovery tools in its actual tool surface | Compared legacy-agent behavior with a fresh agent | Kept prompt/tool-surface gating, but removed legacy full-mode tool backfill from the final change set |
| 2026-03-11 | Anthropic-style `tool_search` flow did not map cleanly onto the current OpenFang loop | Re-read the tool protocol, MCP behavior, and `agent_loop` iteration model | Confirmed the missing piece is automatic expansion of new `ToolDefinition` values into subsequent LLM requests |
| 2026-03-12 | Second top-level live message could not call `github_comment` without re-running discovery | Sent a follow-up message to the same probe agent | Confirmed dynamic expansion is intentionally scoped to a single top-level message / loop lifetime |
| 2026-03-12 | `defer_loading` could not safely hide arbitrary deferred tools yet | Reviewed the current runtime expansion paths while implementing the field | Kept the current generic field, but only skill-provided deferred tools are hidden/expanded until `tool_search` grows beyond skill-backed sources |
| 2026-03-12 | First live `tool_search` alias probe did not perform a real GitHub side effect | Sent an explicit same-message discovery/invocation request to a fresh MiniMax-backed probe agent | Confirmed the alias path works and tool invocation happens; the remaining limitation is that the discovered `github_comment` skill is prompt-only, so execution correctly returns instructional output instead of posting |
| 2026-03-12 | First post-refactor `tool_search` live probe still returned legacy skill-search payloads | Inspected the session log after a successful tool call | Found the daemon was still running a stale CLI binary; rebuilt `openfang-cli`, restarted the daemon, and confirmed the new tool-centric result shape live |
| 2026-03-12 | Need proof that generic `tool_search` handles a real non-skill deferred source | Marked MCP tools as deferred, rebuilt the CLI binary, and ran a fresh MiniMax-backed probe agent with `mcp_servers = ["MiniMax"]` | Confirmed same-message `tool_search -> mcp_minimax_web_search` works live and records agent spend |
| 2026-03-12 | External MCP clients could still see discovery helpers even though they require loop-local `ToolRunner` state | Reviewed `/mcp` request handling and validated the live `tools/list` payload | Filtered `tool_search`, `skill_search`, and `skill_get_instructions` out of `/mcp`; live `tools/list` now exposes only externally meaningful tools |
| 2026-03-12 | `skill_search` was still part of the default discovery surface even though `tool_search` is now the canonical path | Audited profiles, prompt wording, and migration tests | Removed `skill_search` from standard `ToolProfile`s and default prompt guidance while keeping the builtin alias for compatibility |
| 2026-03-12 | `skill_search` compatibility code was still present in runtime and kernel after the public migration to `tool_search` | Audited builtin registry, `ToolRunner`, `KernelHandle`, and prompt plumbing, then reran live surface checks | Deleted the builtin `skill_search` tool and its runtime/kernel support so `tool_search` is now the only discovery entry; `skill_get_instructions` remains |
| 2026-03-12 | `skill_get_instructions` was still outside the unified discovery path after `skill_search` removal | Extended `tool_search` to merge prompt-only/manual skill matches with deferred-tool matches, then validated the new flow live | `tool_search` now returns `skill_manual` entries, and a real MiniMax-backed probe agent completed `tool_search -> skill_get_instructions` in one top-level message |

## 5-Question Reboot Check
| Question | Answer |
|----------|--------|
| Where am I? | Phase 1 discovery with implementation entry points identified |
| Where am I going? | Consolidate the now-live `tool_search` rollout around skill + MCP deferred sources without changing builtin visibility |
| What's the goal? | Preserve the working skill discovery flow while turning `tool_search` into the single runtime-owned deferred discovery surface |
| What have I learned? | `ToolRunner` now owns loop-local visibility for both skill and MCP deferred tools; builtin tools should remain `defer_loading = false`, and a separate `skill_search` alias is no longer needed once `tool_search` fully owns discovery |
| What have I done? | Implemented and validated Phase 1, added loop-local `ToolRunner` ownership, formalized `defer_loading`, introduced `tool_search`, moved it onto a generic hidden-tool search, live-validated same-message expansion for skill and MCP tools, removed `skill_search`, and then folded prompt-only/manual skill loading back into the unified `tool_search -> skill_get_instructions` path |
