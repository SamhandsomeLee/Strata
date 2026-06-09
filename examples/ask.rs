//! Ask DeepSeek a single question via the Strata agentic loop (M1 demo).
//!
//! Usage:
//!   cargo run --example ask -- "你的问题"
//!   echo "你的问题" | cargo run --example ask
//!
//! Requires `.env` with `DEEPSEEK_API_KEY` (see `.env.example`). Trace events go to stderr.

use std::io::{self, BufRead};

use strata::{
    run, ConsoleTracer, DeepSeekProvider, JsonToolCall, Message, Session, ToolRegistry,
};

const MAX_TURNS: u32 = 8;

fn main() {
    if let Err(e) = run_demo() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run_demo() -> Result<(), Box<dyn std::error::Error>> {
    // Load `.env` from crate root — `dotenv()` only walks parents of cwd, so running the
    // binary directly (or from another folder) would miss `E:\...\strata\.env`.
    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenvy::from_path(&env_path).ok();

    let question = read_question()?;
    let provider = DeepSeekProvider::from_env()?;

    let mut session = Session::new();
    session.push(Message::user(question));

    let answer = run(
        &mut session,
        &provider,
        &ToolRegistry::new(),
        &JsonToolCall,
        &ConsoleTracer,
        MAX_TURNS,
    )?;

    println!("{answer}");
    Ok(())
}

fn read_question() -> Result<String, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        return Ok(args.join(" "));
    }

    let mut line = String::new();
    let mut stdin = io::stdin().lock();
    if stdin.read_line(&mut line)? == 0 {
        return Err("usage: cargo run --example ask -- \"your question\"".into());
    }

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("empty question".into());
    }
    Ok(trimmed.to_string())
}
