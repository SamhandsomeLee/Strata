//! Calculator tool demo via the Strata agentic loop (M2 demo).
//!
//! Usage:
//!   cargo run --example calc -- "用 calculator 计算 (17*23)+5"
//!   echo "用 calculator 算 1+2*3" | cargo run --example calc
//!
//! Requires `.env` with `DEEPSEEK_API_KEY` (see `.env.example`). Trace events go to stderr.

use std::io::{self, BufRead};

use strata::{
    run, Calculator, ConsoleTracer, DeepSeekProvider, JsonToolCall, Message, Session, ToolRegistry,
};

const MAX_TURNS: u32 = 8;
const DEFAULT_QUESTION: &str = "用 calculator 工具计算 (17*23)+5，只给出最终数字。";

fn main() {
    if let Err(e) = run_demo() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run_demo() -> Result<(), Box<dyn std::error::Error>> {
    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenvy::from_path(&env_path).ok();

    let question = read_question()?;
    let provider = DeepSeekProvider::from_env()?;

    let mut tools = ToolRegistry::new();
    tools.register(Box::new(Calculator));

    let mut session = Session::new();
    session.push(Message::system(
        "You have a calculator tool. For any arithmetic, you must call it; do not compute in your head.",
    ));
    session.push(Message::user(question));

    let answer = run(
        &mut session,
        &provider,
        &tools,
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
        return Ok(DEFAULT_QUESTION.to_string());
    }

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(DEFAULT_QUESTION.to_string());
    }
    Ok(trimmed.to_string())
}
