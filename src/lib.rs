//! Strata — minimal agent runtime kernel.
//!
//! Module layout follows `doc/strata-runtime-kernel-design.md` §1 (layering) and §2 (contracts).

pub mod action;

pub use action::{Action, ActionBackend, JsonToolCall};
pub mod error;
pub mod message;

pub use error::{LoopError, ParseError, ProviderError, StrataError, ToolError};
pub use message::{ContentBlock, Message, Role};
pub mod provider;

pub use provider::{CompletionRequest, CompletionResponse, Provider};
pub mod providers;

pub use providers::DeepSeekProvider;
pub mod run;

pub use run::run;
pub mod session;

pub use session::Session;
pub mod tool;

pub use tool::{Tool, ToolRegistry, ToolSchema};
pub mod tools;

pub use tools::Calculator;
pub mod trace;

pub use trace::{ConsoleTracer, NoopTracer, TraceEvent, Tracer};
