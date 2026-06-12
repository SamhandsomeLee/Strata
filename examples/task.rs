//! M4 task demo — fix a version mismatch using fs + shell tools (C23).
//!
//! Usage:
//!   cargo run --example task
//!   cargo run --example task -- "自定义任务描述"
//!   cargo run --example task -- --workspace path/to/dir "任务"
//!
//! Copies `examples/fixtures/version-mismatch/` to a temp workspace by default
//! (so fixture files in the repo stay unchanged). Trace events go to stderr.
//!
//! Requires `.env` with `DEEPSEEK_API_KEY` (see `.env.example`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use strata::{
    run, ConsoleTracer, DeepSeekProvider, FsConfig, JsonToolCall, ListDir, LoopError, Message,
    ReadFile, RunCommand, Session, StrataError, ToolRegistry, WriteFile,
};

const MAX_TURNS: u32 = 16;

const FIXTURE_DIR: &str = "examples/fixtures/version-mismatch";

const SYSTEM_PROMPT: &str = "\
You are an assistant working inside a sandboxed workspace directory. \
You have these tools: read_file, write_file, list_dir, run_command. \
Paths for file tools are relative to the workspace root. \
Use run_command for shell verification (platform shell: cmd on Windows, sh on Unix). \
Do not guess file contents — read them first. \
When the task is done, reply with a brief summary of what you changed.";

const DEFAULT_TASK: &str = "\
工作区里有个小项目：README.md 写的是 version 0.2.0，但 app.toml 里的 version 还是 0.1.0。\
请先了解目录结构并读取相关文件，把 app.toml 的 version 改成与 README 一致，\
然后用 run_command 执行一条命令验证两处 version 已一致，最后简短说明你做了什么。";

fn main() {
    if let Err(e) = run_demo() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run_demo() -> Result<(), Box<dyn std::error::Error>> {
    let env_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenvy::from_path(&env_path).ok();

    let (workspace, task) = parse_cli()?;
    eprintln!("workspace: {}", workspace.display());

    let config = FsConfig::new(&workspace)?;
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(ReadFile::new(config.clone())));
    tools.register(Box::new(WriteFile::new(config.clone())));
    tools.register(Box::new(ListDir::new(config.clone())));
    tools.register(Box::new(RunCommand::new(config)));

    let provider = DeepSeekProvider::from_env()?;

    let mut session = Session::new();
    session.push(Message::system(SYSTEM_PROMPT));
    session.push(Message::user(task));

    let result = run(
        &mut session,
        &provider,
        &tools,
        &JsonToolCall,
        &ConsoleTracer,
        MAX_TURNS,
    );

    match result {
        Ok(answer) => {
            println!("{answer}");
            verify_fix(&workspace);
            Ok(())
        }
        Err(StrataError::Loop(LoopError::MaxTurns { max_turns, partial })) => {
            eprintln!("error: 达到最大轮数 {max_turns}，未能得出最终回答");
            if let Some(text) = partial {
                eprintln!("--- 部分结果 ---");
                println!("{text}");
            }
            verify_fix(&workspace);
            std::process::exit(1);
        }
        Err(e) => Err(e.into()),
    }
}

struct ParsedCli {
    workspace: PathBuf,
    task: String,
}

fn parse_cli() -> Result<(PathBuf, String), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = parse_args(&args)?;
    Ok((parsed.workspace, parsed.task))
}

fn parse_args(args: &[String]) -> Result<ParsedCli, Box<dyn std::error::Error>> {
    let mut workspace: Option<PathBuf> = None;
    let mut task_parts: Vec<String> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--workspace" => {
                i += 1;
                let path = args
                    .get(i)
                    .ok_or("--workspace requires a path")?
                    .clone();
                workspace = Some(PathBuf::from(path));
                i += 1;
            }
            other => {
                task_parts.push(other.to_string());
                i += 1;
                while i < args.len() {
                    task_parts.push(args[i].clone());
                    i += 1;
                }
            }
        }
    }

    let workspace = match workspace {
        Some(path) => path,
        None => copy_fixture_to_temp()?,
    };

    let task = if task_parts.is_empty() {
        DEFAULT_TASK.to_string()
    } else {
        task_parts.join(" ")
    };

    Ok(ParsedCli { workspace, task })
}

fn copy_fixture_to_temp() -> Result<PathBuf, io::Error> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dest = std::env::temp_dir().join(format!("strata_task_{stamp}"));
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR);
    copy_dir_all(&src, &dest)?;
    Ok(dest)
}

fn copy_dir_all(src: &Path, dest: &Path) -> Result<(), io::Error> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn verify_fix(workspace: &Path) {
    let app_toml = workspace.join("app.toml");
    match fs::read_to_string(&app_toml) {
        Ok(text) if text.contains("0.2.0") => {
            eprintln!("task verify: ok (app.toml contains 0.2.0)");
        }
        Ok(_) => {
            eprintln!("task verify: warn — app.toml may still have wrong version");
        }
        Err(e) => {
            eprintln!("task verify: skip — could not read app.toml: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_task_when_no_args() {
        let parsed = parse_args(&[]).expect("parse");
        assert!(parsed.task.contains("0.2.0"));
        assert!(parsed.workspace.exists());
        let _ = fs::remove_dir_all(&parsed.workspace);
    }

    #[test]
    fn custom_task_from_args() {
        let args = vec!["hello".into(), "world".into()];
        let parsed = parse_args(&args).expect("parse");
        assert_eq!(parsed.task, "hello world");
        let _ = fs::remove_dir_all(&parsed.workspace);
    }

    #[test]
    fn workspace_flag_uses_given_dir() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("strata_task_cli_{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");

        let args = vec![
            "--workspace".into(),
            dir.to_string_lossy().into_owned(),
            "do it".into(),
        ];
        let parsed = parse_args(&args).expect("parse");
        assert_eq!(parsed.workspace, dir);
        assert_eq!(parsed.task, "do it");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fixture_copy_contains_expected_files() {
        let workspace = copy_fixture_to_temp().expect("copy");
        assert!(workspace.join("README.md").is_file());
        assert!(workspace.join("app.toml").is_file());
        let toml = fs::read_to_string(workspace.join("app.toml")).expect("read");
        assert!(toml.contains("0.1.0"));
        let _ = fs::remove_dir_all(&workspace);
    }
}
