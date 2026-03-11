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
  - Verified `/api/health`, `/api/tools`, and `/api/agents` against the configured local API port `50051`.
  - Confirmed `/api/tools` now exposes builtin discovery tools plus skill-provided executable tools.
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
- Files created/modified:
  - `docs/skill-progressive-loading-design.md`
  - `progress.md`
  - `task_plan.md`
  - `findings.md`
## Test Results
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Registry search | `cargo test -q -p openfang-skills registry::tests::test_search_matches_name_and_tags -- --exact` | New lexical search test passes | Passed | ✓ |
| Prompt protocol | `cargo test -q -p openfang-runtime prompt_builder::tests::test_skills_section_present -- --exact` | Prompt advertises protocol instead of catalog | Passed | ✓ |
| Builtin tool list | `cargo test -q -p openfang-runtime tool_runner::tests::test_builtin_tool_definitions -- --exact` | Builtin tool registry includes `skill_search` | Passed | ✓ |
| Workspace build | `cargo build --workspace --lib` | Workspace libraries compile | Passed | ✓ |
| Workspace tests | `cargo test --workspace` | Full test suite passes | Passed | ✓ |
| Workspace clippy | `cargo clippy --workspace --all-targets -- -D warnings` | Zero warnings | Passed | ✓ |
| Live health check | `curl -s http://127.0.0.1:50051/api/health` | API returns healthy status | `{"status":"ok","version":"0.3.34"}` | ✓ |
| Live tools check | `curl -s http://127.0.0.1:50051/api/tools` | Discovery tools and skill callable tools are visible | Included `skill_search`, `skill_get_instructions`, and skill tool `node_live_check` | ✓ |
| Live agents check | `curl -s http://127.0.0.1:50051/api/agents` | API responds with agent list | Returned local `assistant` agent with `auth_status:"missing"` | ✓ |
| MiniMax skill-discovery live check | Fresh `skill-probe` agent on `127.0.0.1:50124` | Model should discover and load a relevant skill before acting | Observed `skill_search`, then `skill_get_instructions("github")`, then execution guided by the skill | ✓ |
| MiniMax skill-discovery re-check without XML recovery | Fresh `skill-probe-xml-off` agent on `127.0.0.1:50124` | Clean-agent flow should still work if XML recovery is unnecessary | Observed `skill_search`, then `skill_get_instructions("github")`, then `shell_exec` without XML recovery logic present | ✓ |

## Error Log
| Timestamp | Error | Attempt | Resolution |
| 2026-03-11 | `openfang-migrate` profile-count test failed after tool-profile expansion | Ran full workspace tests | Updated test to assert `skill_search` and `skill_get_instructions` presence |
| 2026-03-11 | Prompt builder ordering/soul tests failed after static-skill rewrite | Ran full workspace tests | Restored a stable `## Tone` section format in `build_soul_section()` |
| 2026-03-11 | Real live LLM message test could not run | Checked environment and daemon auth state | Blocked because `GROQ_API_KEY` is unset locally |
| 2026-03-11 | Initial MiniMax failure sample suggested XML-style text tool calls might require recovery | Re-ran the same scenario on a fresh agent after isolating variables | Determined XML recovery was not required for the Phase 1 flow and removed it |
| 2026-03-11 | Existing legacy `assistant` agent exposed `## Skills` in prompt while missing discovery tools in its actual tool surface | Compared legacy-agent behavior with a fresh agent | Kept prompt/tool-surface gating, but removed legacy full-mode tool backfill from the final change set |
| 2026-03-11 | Anthropic-style `tool_search` flow did not map cleanly onto the current OpenFang loop | Re-read the tool protocol, MCP behavior, and `agent_loop` iteration model | Confirmed the missing piece is automatic expansion of new `ToolDefinition` values into subsequent LLM requests |
|-----------|-------|---------|------------|

## 5-Question Reboot Check
| Question | Answer |
|----------|--------|
| Where am I? | Phase 1 discovery with implementation entry points identified |
| Where am I going? | Next-stage design review for Anthropic-compatible `tool_search` with automatic tool expansion |
| What's the goal? | Preserve the completed Phase 1 behavior while redesigning the next phase around `tool_search`, `defer_loading`, and dynamic tool expansion |
| What have I learned? | LLMs already receive unified `ToolDefinition` objects for builtin, skill, and MCP, but the current `agent_loop` cannot dynamically expand the tool set after a search result |
| What have I done? | Implemented and validated Phase 1, removed non-essential compatibility patches, then updated the design docs to capture the next-stage constraints and the current `agent_loop` gap |
