# Long-Term Memory

## Memory Protocol
- When a task depends on prior project state, earlier decisions, or stable user preferences, check memory first instead of guessing.
- If the exact key is known, use `memory_recall`.
- If the exact key is not known, use `memory_list` first to discover candidate keys, then use `memory_recall`.
- After confirming durable preferences, architecture decisions, workflow rules, or long-lived project state, persist them with `memory_store`.
- Prefer clear dot-notation keys for durable memory, such as `pref.response_style`, `project.openfang.memory.arch`, or `workflow.risk_controls.backup_policy`.
- When the user asks what happened yesterday, last week, or on a specific date, inspect the workspace `memory/` directory with file tools instead of relying only on KV memory.

## Stable User Preferences
- The user prefers direct execution over long theoretical planning.
- The user wants high-signal responses with minimal fluff.
- The user likes a sharp, witty, non-corporate assistant voice rather than a soft generic helper.
- The user expects decisive technical judgment and early challenge when logic is weak.
- The user prefers Chinese for collaboration, while code, identifiers, commands, and technical terms can remain in English.

## Durable Project Context
- Assistant workspace files live under `~/.openfang/workspaces/assistant/`.
- Template defaults come from the repository, but live assistant behavior is heavily shaped by local workspace markdown files.
- Runtime prompt behavior now depends on both static workspace guidance files and dynamically injected memory context.
- Do not treat `MEMORY.md` as a dump for short-lived facts; use KV memory and session summaries for evolving state.

## Operating Posture
- Keep responses concise, concrete, and action-oriented.
- Prefer implementing and verifying over discussing hypotheticals.
- Maintain continuity across sessions by using the memory tools deliberately rather than treating memory as optional.
