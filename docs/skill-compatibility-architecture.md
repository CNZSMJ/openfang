# Skill Compatibility Architecture

Status: proposed

## Purpose

This document defines the target architecture for OpenFang's skill system so it can:

1. Fully interoperate with the OpenClaw skill ecosystem.
2. Fully interoperate with Anthropic / Claude Code style skills built around `SKILL.md`.
3. Preserve OpenFang's existing agent permission model instead of bypassing it.
4. Support future third-party skill sources without creating one-off compatibility paths.

This is an architecture and implementation design document. It describes the target state, the problems in the current implementation, and the technical decisions needed to reach that target.

## Problem Background

OpenFang already has a `openfang-skills` crate, a skill registry, prompt injection for prompt-only skills, and runtime dispatch for Python and Node skills. However, the current design has four structural problems:

1. The system mixes knowledge skills and executable skills under one loose model.
2. OpenClaw compatibility is partial and often degrades executable semantics into prompt-only behavior.
3. Installation, loading, listing, and execution are implemented across multiple paths with inconsistent behavior.
4. Documentation and implementation have drifted, especially around installation, bundled skills, and WASM runtime support.

The result is that OpenFang currently supports some skill-shaped content, but it does not yet provide a coherent compatibility layer for either OpenClaw or Anthropic's native skill model.

## External Ecosystems We Must Support

### Anthropic / Claude Code / `anthropics/skills`

Anthropic's public skills repository and related documentation establish a minimal skill contract:

- A skill is a directory.
- The directory contains a `SKILL.md`.
- The frontmatter requires only `name` and `description`.
- The body contains instructions, examples, and guidelines.
- A skill may include scripts, templates, references, and other resources.
- Skills are dynamically loaded and selected by the model.

This is the base `SKILL.md` ecosystem that OpenFang must support.

### OpenClaw

OpenClaw uses the same `SKILL.md` core concept but adds platform-specific behavior:

- Load precedence across bundled, user, and workspace skill directories.
- Optional extra load directories.
- Eligibility and readiness checks.
- Optional frontmatter such as `user-invocable`, `disable-model-invocation`, `command-dispatch`, `command-tool`, and `command-arg-mode`.
- Environment and binary requirements.
- Platform-level command invocation semantics.

OpenClaw is therefore best modeled as:

`Anthropic-style SKILL.md core + OpenClaw-specific extensions`

## External References

These sources define the ecosystems this design targets:

- Anthropic public skills repository: `anthropics/skills`
- Anthropic Claude Code and agent skills documentation
- OpenClaw skills documentation
- OpenClaw custom skill documentation

The compatibility target in this document is based on those published contracts, not on undocumented behavior.

## Current OpenFang State

Current verified behavior in the codebase:

- `SkillManifest` models runtime, provided tools, requirements, prompt context, and source.
- `PromptOnly` skills influence the model mainly through prompt context.
- `Python` and `Node` skills can execute as subprocesses.
- `Wasm` is present in the type system but not implemented in the loader.
- `openclaw_compat.rs` converts `SKILL.md` skills into `PromptOnly` manifests, even when commands are present.
- `tool_runner.rs` dispatches to a skill provider if a tool name is found in the skill registry.
- Agent manifests already include `skills`, `tool_allowlist`, and `tool_blocklist`.

This gives OpenFang some primitives, but not a unified compatibility architecture.

## Design Goals

### In Scope

- Full compatibility with Anthropic-style `SKILL.md` skills.
- Full compatibility with OpenClaw skill discovery, installation, metadata, and invocation semantics where they can be safely expressed inside OpenFang.
- Preservation of OpenFang's agent permission model.
- Support for local, workspace, marketplace, and GitHub-hosted third-party skills.
- Clear rejection behavior for unsupported or unsafe skills.

### Out of Scope

- Executing arbitrary repositories that do not conform to a supported skill format.
- Weakening agent tool or capability restrictions in order to match OpenClaw behavior.
- Silent best-effort degradation for unsupported semantics.

## Core Architectural Decision

OpenFang will use a single canonical internal skill model with adapter layers for external ecosystems.

This is the central design choice that all other decisions depend on.

### Why

Without a canonical model, every source format creates a new branch of special-case behavior:

- Anthropic skills become one parser path.
- OpenClaw skills become another parser path.
- Native OpenFang skills become a third path.

That produces drift, duplicated bugs, and inconsistent installation and execution behavior.

### Decision

Adopt this layering:

1. Source adapter
2. Format adapter
3. Canonical OpenFang skill model
4. Authorization filter
5. Runtime dispatcher

The canonical model is the only representation used by the registry, API, kernel, and runtime.

## Technical Decisions

### Decision 1: Canonical Skill Model Lives in `openfang-skills`

#### Background

The compatibility data belongs to the skill system, not to the agent manifest and not to the runtime driver.

#### Decision

Keep the canonical skill model in `crates/openfang-skills`, extending the existing core types in `lib.rs` and adding a dedicated module for policy and compatibility metadata.

Planned modules:

- `crates/openfang-skills/src/lib.rs`
- `crates/openfang-skills/src/policy.rs`
- `crates/openfang-skills/src/installer.rs`
- `crates/openfang-skills/src/adapters/anthropic.rs`
- `crates/openfang-skills/src/adapters/openclaw.rs`

#### Consequences

- OpenClaw and Anthropic compatibility become normal adapters instead of hard-coded exceptions.
- Kernel, API, and runtime consume one stable internal representation.

### Decision 2: Separate Core Skill Shape from Ecosystem Extensions

#### Background

Anthropic skills and OpenClaw skills share a common `SKILL.md` base, but OpenClaw adds platform-specific metadata and behavior.

#### Decision

Represent skills internally as:

- A canonical core
- Zero or more ecosystem extensions

Canonical core fields:

- identity: `name`, `description`, `version`, `source`
- location: install path, scope, precedence
- content: instructions body, prompt context, resources
- execution: runtime type, entrypoint, provided tools
- host requirements: tools, capabilities, binaries, environment variables

Extension blocks:

- `AnthropicSkillExtension`
- `OpenClawSkillExtension`

The core is mandatory. Extensions are optional.

#### Consequences

- Anthropic skills do not need to fake OpenClaw-only metadata.
- OpenClaw-specific semantics are preserved instead of flattened away.

### Decision 3: Add a Strongly Typed `SkillCommandPolicy`

#### Background

OpenClaw exposes command-level behavior such as `user-invocable` and `disable-model-invocation`. OpenFang currently parses pieces of that metadata but does not carry it as a first-class internal concept.

#### Decision

Add a new internal type in `crates/openfang-skills/src/policy.rs` and attach it to each skill-provided tool.

Suggested shape:

```rust
pub enum CommandInvoker {
    Model,
    User,
    Both,
}

pub enum CommandDispatchMode {
    ModelMediated,
    DirectTool,
}

pub struct SkillCommandPolicy {
    pub invoker: CommandInvoker,
    pub dispatch_mode: CommandDispatchMode,
    pub requires_confirmation: bool,
    pub raw_arg_mode: bool,
    pub host_tools: Vec<String>,
    pub host_capabilities: Vec<String>,
}
```

`SkillToolDef` becomes:

```rust
pub struct SkillToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub policy: SkillCommandPolicy,
}
```

#### Why This Belongs Here

- It describes the skill command itself.
- It originates from skill metadata.
- It must be available during installation, listing, prompt exposure, and runtime dispatch.

#### Consequences

- OpenClaw metadata is preserved.
- Anthropic-style skills can use a safe default policy.
- Runtime authorization becomes explicit instead of inferred from tool names.

#### Default Mapping Rules

When OpenClaw metadata is missing, the canonical policy uses explicit defaults:

- missing `user-invocable` -> treated as user-invocable
- missing `disable-model-invocation` -> treated as model-invocable
- missing dispatch metadata -> `ModelMediated`
- missing arg mode metadata -> structured arguments, not raw passthrough

Anthropic-style skills without OpenClaw extensions use these same defaults.

### Decision 4: Treat `AgentManifest.skills` as the Skill Allowlist

#### Background

OpenFang already has `skills`, `tool_allowlist`, and `tool_blocklist` in `AgentManifest`.

#### Decision

Do not add a new `allowed_skills` field. Use the existing `skills` field as the formal skill allowlist.

Permission evaluation becomes:

`effective_skill_visibility = agent.skills allowlist AND tool allow/block filters AND skill host requirements AND command policy`

#### Why

The current manifest already has the right abstraction; it is simply not enforced deeply enough across the whole lifecycle.

#### Consequences

- Minimal manifest churn.
- Stronger compatibility with current OpenFang agents.
- No redundant config surface.

### Decision 5: Agent Tool Restrictions Remain the Hard Upper Bound

#### Background

A compatibility layer must not weaken OpenFang's security model.

#### Decision

A skill command can only be visible and executable if the current agent's manifest and effective capabilities allow the host tools and capabilities required by that command.

Examples:

- A skill command needing `shell_exec` is unavailable to an agent without `shell_exec`.
- A prompt-only skill can still be blocked by `skills` allowlist even if it needs no host tool.

#### Consequences

- Compatibility is semantic, not permissive.
- OpenClaw skills are adapted into OpenFang's permission model rather than bypassing it.

### Decision 6: Use a Unified Installer Pipeline

#### Background

The current installation behavior is fragmented:

- Marketplace install is incomplete.
- Local path install is stubbed.
- API and kernel flows do not share the same materialization behavior.

#### Decision

All installation paths must use the same pipeline:

1. Fetch or read source
2. Detect format
3. Parse metadata
4. Verify security and dependencies
5. Convert to canonical model
6. Materialize into OpenFang install layout
7. Reload registry

Supported source kinds:

- local directory
- local archive
- GitHub repository or release
- ClawHub slug
- future third-party source adapters

Supported formats:

- OpenFang native manifest
- Anthropic-style `SKILL.md`
- OpenClaw `SKILL.md`
- OpenClaw Node skill

#### Consequences

- No more partial install states.
- All compatibility logic is concentrated in one layer.

### Decision 7: Reject Unsupported or Unsafe Skills at Install Time

#### Background

Silent degradation causes users to think a skill is available when it is not.

#### Decision

If a skill cannot be safely expressed or executed inside OpenFang, the installer must reject it and explain why.

Install outcomes:

- `installed`
- `rejected`
- `needs_manual_action`

Typical rejection reasons:

- unsupported runtime
- unsupported invocation semantics
- required binaries missing
- required environment variables missing
- host capability mapping impossible
- prompt injection or security scan failure

#### Consequences

- Better trust in the skill system
- Clear operator feedback
- Safer compatibility boundary

### Decision 8: Split Knowledge Skills and Executable Skills Conceptually

#### Background

OpenFang currently treats prompt-only and executable skills as the same kind of thing, which obscures runtime behavior.

#### Decision

Keep a unified manifest type, but explicitly classify skills at runtime as:

- `knowledge skill`
- `executable skill`
- `hybrid skill`

Definitions:

- knowledge skill: prompt/manual only
- executable skill: tools and runtime behavior only
- hybrid skill: both instructions and executable commands

#### Consequences

- Anthropic skills are naturally modeled as knowledge or hybrid skills.
- OpenClaw skills with command metadata can remain hybrid instead of being flattened to prompt-only.

### Decision 9: Support OpenClaw Load Precedence and Discovery Semantics

#### Background

OpenClaw compatibility is not only about file format. It also includes directory precedence and discovery behavior.

#### Decision

OpenFang will support these precedence levels for compatible skills:

1. workspace skills
2. user-installed skills
3. bundled skills
4. optional extra directories with lowest precedence

This precedence is applied before the agent-specific `skills` allowlist.

#### Consequences

- Workspace overrides behave as OpenClaw users expect.
- Bundled skills remain the lowest-priority defaults.

### Decision 10: Prompt Exposure Must Respect Policy and Permissions

#### Background

Today, skill prompt exposure and tool exposure are loosely connected.

#### Decision

When constructing the system prompt and tool list for an agent:

- only expose skills allowed by `AgentManifest.skills`
- only expose tools whose `SkillCommandPolicy` permits model invocation
- only expose tools whose host requirements fit the agent's capabilities

User-only commands remain callable from explicit UI or CLI user entrypoints, but they are not advertised to the model as normal tools.

#### Consequences

- The model sees only the skills and commands it can actually use.
- Fewer false affordances in prompts.

### Decision 11: Runtime Dispatch Must Re-check Authorization

#### Background

Prompt filtering alone is insufficient. A dispatch path can still be reached indirectly.

#### Decision

Before executing a skill tool, runtime dispatch must validate:

- the skill is installed and enabled
- the skill is allowed for the current agent
- the command policy permits the current caller
- the agent has the required host tools and capabilities

This validation happens even if the tool was already shown to the model.

#### Consequences

- Defense in depth
- Safer handling of registry or prompt bugs

### Decision 12: WASM Is Not Part of Compatibility Scope Until It Exists

#### Background

WASM is declared in the current type system but not implemented in the loader.

#### Decision

Compatibility work will not depend on WASM. During this effort:

- either implement real WASM execution
- or document it as unsupported and remove claims that it is already available

#### Consequences

- The compatibility plan is grounded in actual runtime behavior.
- The doc surface stops overstating current support.

## Canonical Lifecycle

Every skill, regardless of source, must move through the same lifecycle:

1. source discovery
2. source fetch or read
3. format detection
4. metadata parse
5. dependency and security verification
6. canonical conversion
7. install materialization
8. registry load
9. agent-specific authorization
10. prompt or tool exposure
11. runtime execution

This lifecycle is the compatibility contract.

## Example: Anthropic Skill vs OpenClaw Skill

### Anthropic-style skill

Input:

- folder
- `SKILL.md`
- optional scripts or templates

Result:

- canonical skill core
- default `SkillCommandPolicy`
- usually knowledge skill or hybrid skill

### OpenClaw skill

Input:

- folder
- `SKILL.md`
- optional OpenClaw frontmatter extensions
- optional executable code or tool dispatch metadata

Result:

- canonical skill core
- `OpenClawSkillExtension`
- preserved `SkillCommandPolicy`
- knowledge, executable, or hybrid classification

## API and CLI Impact

The new architecture requires the API and CLI to expose more state.

Recommended skill metadata in list/detail responses:

- `kind`: knowledge | executable | hybrid
- `runtime`
- `source`
- `scope`
- `allowed_for_agent`
- `blocked_reason`
- `model_invocable`
- `user_invocable`
- `required_binaries`
- `required_host_tools`

This is necessary to explain why a skill is installed but not available to a given agent.

## Migration Strategy

### Phase 1: Canonical model and installer

- add policy and adapter types
- unify install paths
- preserve current runtime behavior

### Phase 2: Authorization and prompt alignment

- enforce `skills` allowlist
- enforce command policy during exposure
- enforce runtime re-checks

### Phase 3: External ecosystem completion

- complete OpenClaw precedence and discovery semantics
- support Anthropic-style `SKILL.md` packages as first-class inputs
- improve detail APIs and CLI reporting

### Phase 4: Documentation and verification

- update `skill-development.md`
- update `api-reference.md`
- add compatibility tests and live integration coverage

## Risks

### Risk: Too much compatibility logic in the parser

Mitigation:

- keep source parsing separate from canonical conversion

### Risk: Policy duplication across skill, agent, and runtime

Mitigation:

- define policy once in `openfang-skills`
- consume it from kernel and runtime

### Risk: Prompt-only skills leaking into unauthorized agents

Mitigation:

- always apply `AgentManifest.skills` allowlist before prompt exposure

### Risk: False claims of compatibility

Mitigation:

- reject unsupported skills instead of silently downgrading them

## Final Position

OpenFang should not implement OpenClaw compatibility as a special parser hack. It should implement a canonical skill architecture that:

- treats Anthropic `SKILL.md` as the base format
- treats OpenClaw as a compatible extension layer
- preserves OpenFang's existing agent permission system
- rejects unsupported semantics explicitly

That is the only design that can support both ecosystems without turning the skill system into an unmaintainable set of exceptions.
