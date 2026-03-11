# Task Plan: Skill Progressive Loading Phase 1 / Tool Search Follow-Up

## Goal
Complete and preserve Phase 1 of the progressive loading design, then define the next-stage Anthropic-compatible `tool_search` direction around `defer_loading` and automatic tool expansion.

## Current Phase
Phase 6

## Phases

### Phase 1: Requirements & Discovery
- [x] Understand user intent
- [x] Identify constraints and requirements
- [x] Document findings in findings.md
- **Status:** complete

### Phase 2: Planning & Structure
- [x] Define technical approach
- [x] Identify touched runtime/API/test surfaces
- [x] Document decisions with rationale
- **Status:** complete

### Phase 3: Implementation
- [x] Add local runtime skill discovery
- [x] Update prompt strategy to use discovery instead of full catalog
- [x] Unify `/api/tools` skill visibility
- [x] Add or update tests
- **Status:** complete

### Phase 4: Testing & Verification
- [x] Run `cargo build --workspace --lib`
- [x] Run `cargo test --workspace`
- [x] Run `cargo clippy --workspace --all-targets -- -D warnings`
- [x] Run live integration validation for affected API/runtime behavior
- **Status:** complete

### Phase 5: Delivery
- [x] Review modified files
- [x] Summarize behavior changes and verification
- [x] Deliver outcome and residual risks
- **Status:** complete

### Phase 6: Anthropic-Compatible Design Review
- [x] Reconstruct the current builtin / skill / MCP tool-call path
- [x] Confirm whether the LLM directly receives `ToolDefinition`
- [x] Compare current OpenFang flow with Anthropic `tool_search`
- [x] Identify where current `agent_loop` blocks automatic tool expansion
- [x] Record the minimal next-stage design direction in docs
- [ ] Design the runtime changes needed for dynamic tool expansion in `agent_loop`
- **Status:** in progress

## Key Questions
1. Where should skill discovery live so the runtime, prompting, and API can share the same local skill view?
2. How much of the existing skill metadata should remain in prompt context after removing the full catalog?
3. Should `/api/tools` surface instructional skill resources separately from callable tools?
4. How should OpenFang evolve from `skill_search` to Anthropic-compatible `tool_search` without breaking the current working Phase 1 flow?
5. How should `agent_loop` update its tool set after a search result so a later LLM turn can actually call newly expanded tools?

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Use the existing Phase 1 names `skill_search` and `skill_get_instructions` | The design explicitly says v1 should change structure, not external naming |
| Treat this change as a skills-only rollout | The design scopes Phase 1 to bundled, user-installed, and workspace skills only |
| Make the prompt carry only a skill protocol, not per-skill summaries | This removes static catalog token cost while preserving discoverability |
| Enforce skill visibility in kernel-backed search | Agent skill allowlists remain the hard visibility boundary |
| Keep Anthropic compatibility as the next-stage target, not a retrofit into Phase 1 semantics | Phase 1 is working and validated; the next change should solve the broader tool-loading problem without muddying the completed rollout |
| Treat `ToolDefinition` as the current unified LLM-facing truth across builtin, skill, and MCP | The LLM already receives all three categories as `ToolDefinition` instances today |
| Avoid introducing a heavy `ToolCatalogEntry + metadata` abstraction before proving it is needed | The next concrete blocker is dynamic tool expansion in `agent_loop`, not catalog metadata richness |
| The next-stage canonical flow should be `llm -> tool_search -> automatic expansion -> llm tool_use -> ToolCall` | This best matches Anthropic's model and the current discussion outcome |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| `openfang-migrate` profile test assumed old tool counts | Full workspace test failed after discovery tools were added to standard profiles | Updated the test to assert presence of discovery tools instead of old counts |
| `prompt_builder` soul-section tests failed in full workspace test run | Static skills rewrite exposed existing formatting assumptions in prompt tests | Restored a stable `## Tone` section shape so ordering and soul tests pass |
| Initial MiniMax failure sample appeared to justify XML text-tool-call recovery | Re-tested on a fresh MiniMax-backed agent after isolating the prior legacy-agent mismatch | Determined XML recovery was not required and removed it from the final patch |
| Legacy full-mode `assistant` mismatch made `## Skills` appear even when discovery tools were absent | Compared the legacy agent with a fresh clean agent | Kept the prompt/tool-surface gating fix, but removed the legacy full-mode backfill from the final patch |
| Anthropic-style thin `tool_reference` initially seemed incompatible with OpenFang | Reconstructed the real current tool loop and checked where tools are injected into the LLM request | Determined the real blocker is that `agent_loop` currently keeps a fixed `available_tools` slice across iterations and cannot auto-expand new `ToolDefinition` values |

## Notes
- Do not touch `openfang-cli`.
- After implementation, run build, test, clippy, and live integration validation per AGENTS.md.
- Live daemon validation used the configured local API port `50051`, not the documented default `4200`.
- Real LLM live-call verification is still blocked locally because `GROQ_API_KEY` is not set in this shell.
- Current review conclusion: the LLM already receives full `ToolDefinition` objects for builtin, skill, and MCP; the next-stage design work should focus on deferred exposure and dynamic expansion rather than inventing a richer external `tool_reference`.
