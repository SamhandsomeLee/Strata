//! Strata — minimal agent runtime kernel.
//!
//! Module layout follows `doc/strata-runtime-kernel-design.md` §1 (layering) and §2 (contracts).

pub mod action;
pub mod error;
pub mod message;

pub use message::{ContentBlock, Message, Role};
pub mod provider;
pub mod run;
pub mod session;
pub mod tool;
pub mod trace;
