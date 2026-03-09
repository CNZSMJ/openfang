use serde::{Deserialize, Serialize};

/// Who may invoke a skill command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommandInvoker {
    /// The LLM may invoke the command directly.
    #[default]
    Model,
    /// Only the user may invoke the command through an explicit UI/CLI path.
    User,
    /// Both the LLM and the user may invoke the command.
    Both,
}

/// How a skill command should be dispatched once selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommandDispatchMode {
    /// The model invokes the command through the normal tool-use flow.
    #[default]
    ModelMediated,
    /// The host should trigger the command directly without exposing it as a
    /// normal LLM tool. Reserved for future explicit user-entrypoint flows.
    DirectTool,
}

/// Policy attached to each skill-provided tool or command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SkillCommandPolicy {
    /// Who may invoke this command.
    pub invoker: CommandInvoker,
    /// How the command is dispatched.
    pub dispatch_mode: CommandDispatchMode,
    /// Whether explicit confirmation is required before execution.
    pub requires_confirmation: bool,
    /// Whether the command expects raw string passthrough instead of structured args.
    pub raw_arg_mode: bool,
    /// Host tools this command depends on.
    pub host_tools: Vec<String>,
    /// Host capabilities this command depends on.
    pub host_capabilities: Vec<String>,
}

impl SkillCommandPolicy {
    /// True when the LLM may invoke the command in normal tool-use flow.
    pub fn model_invocable(&self) -> bool {
        matches!(self.invoker, CommandInvoker::Model | CommandInvoker::Both)
    }

    /// True when an explicit user action may invoke the command.
    pub fn user_invocable(&self) -> bool {
        matches!(self.invoker, CommandInvoker::User | CommandInvoker::Both)
    }
}
