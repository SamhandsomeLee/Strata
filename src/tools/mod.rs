//! Concrete [`Tool`](crate::Tool) implementations (design §2.3).
//!
//! Executable tools live here; the kernel defines only the trait boundary in `tool.rs`.

pub mod calculator;
pub mod fs;
pub mod shell;

pub use calculator::Calculator;
pub use fs::{FsConfig, ListDir, ReadFile, WriteFile, MAX_READ_BYTES};
pub use shell::{RunCommand, DEFAULT_TIMEOUT_SECS, MAX_OUTPUT_BYTES};
