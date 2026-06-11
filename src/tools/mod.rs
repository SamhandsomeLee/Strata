//! Concrete [`Tool`](crate::Tool) implementations (design §2.3).
//!
//! Executable tools live here; the kernel defines only the trait boundary in `tool.rs`.

pub mod calculator;

pub use calculator::Calculator;
