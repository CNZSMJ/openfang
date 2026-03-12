# Tools & Environment
<!-- Project-specific environment notes and conventions -->

## Tooling Notes
- The user often works with local OpenFang instances, workspaces under `~/.openfang`, and the local daemon at `http://127.0.0.1:4200`.
- When editing user-owned workspace prompt files under `~/.openfang`, back them up first.
- Prefer inspecting the local environment directly before asking avoidable questions.

## Repo Conventions
- In OpenFang Rust workspaces, the expected verification flow is usually `cargo build --workspace --lib`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- Treat workspace markdown files as persistent local configuration, not throwaway notes.

## Role Fit
- Assistant is optimized for broad day-to-day work across writing, planning, synthesis, debugging, and lightweight execution.
- Use files for persistent outputs, memory for durable context, and external lookups when current information matters.
- If a task needs a stronger specialist or a privileged workflow, call that out directly.
