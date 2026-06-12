//! Ask DeepSeek a single question via the Strata agentic loop (M1 demo).
//!
//! Usage:
//!   cargo run --example ask -- "你的问题"
//!   echo "你的问题" | cargo run --example ask
//!
//! Requires `.env` with `DEEPSEEK_API_KEY` (see `.env.example`). Trace events go to stderr.

use std::io::{self, BufRead};

use strata::{
    run, ConsoleTracer, DeepSeekProvider, JsonToolCall, LoopError, Message, Session, StrataError,
    ToolRegistry,
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

    let result = run(
        &mut session,
        &provider,
        &ToolRegistry::new(),
        &JsonToolCall,
        &ConsoleTracer,
        MAX_TURNS,
    );

    match result {
        Ok(answer) => {
            println!("{answer}");
            Ok(())
        }
        // `Display` 上的 MaxTurns 只是摘要，partial 在结构体字段里——单独取出展示部分结果。
        Err(StrataError::Loop(LoopError::MaxTurns { max_turns, partial })) => {
            eprintln!("error: 达到最大轮数 {max_turns}，未能得出最终回答");
            if let Some(text) = partial {
                eprintln!("--- 部分结果 ---");
                println!("{text}");
            }
            std::process::exit(1);
        }
        Err(e) => Err(e.into()),
    }
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
