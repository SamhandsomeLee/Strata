//! Workspace-scoped shell command tool (design §5, C22).
//!
//! Runs a command via `sh -c` (Unix) or `cmd /C` (Windows) with cwd under the workspace root.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::ToolError;
use crate::tool::{Tool, ToolSchema};
use crate::tools::fs::FsConfig;

/// Default wall-clock limit per command (seconds).
///
/// 记录：MVP 固定值，schema 不提供 per-call 覆盖；将来如需可加 `timeout_secs` 参数。
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Alias aligned with [`MAX_READ_BYTES`] — per-stream capture limit.
pub use crate::tools::fs::MAX_READ_BYTES as MAX_OUTPUT_BYTES;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Shell command runner scoped to a workspace root.
///
/// 安全边界（记录）：`cwd` 受 workspace 限制，但**命令本身不受限**——它是任意代码执行，
/// 可以 `cat /etc/passwd`、`cd / && ...` 等逃逸出 workspace。这是 MVP 的有意取舍：设计 §5
/// 把 bash/shell 列入 MVP，而 CodeAction 沙箱才是「明确不做」。cwd 限制是便利，不是隔离。
#[derive(Debug, Clone)]
pub struct RunCommand {
    config: FsConfig,
}

impl RunCommand {
    pub fn new(config: FsConfig) -> Self {
        Self { config }
    }
}

impl Tool for RunCommand {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "run_command".into(),
            description: "Run a shell command in the workspace. Uses sh on Unix and cmd on Windows. Returns exit_code, stdout, and stderr.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command string, e.g. \"cargo test\" or \"dir /b\""
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory relative to workspace root. Default \".\""
                    }
                },
                "required": ["command"]
            }),
        }
    }

    fn execute(&self, args: serde_json::Value) -> Result<String, ToolError> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidArgs("missing or empty required field: command".into()))?;

        let cwd_raw = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(".");

        let cwd = self.config.resolve(cwd_raw)?;
        if !cwd.is_dir() {
            return Err(ToolError::ExecutionFailed(format!(
                "not a directory: {cwd_raw}"
            )));
        }

        run_shell_command(command, &cwd)
    }
}

fn run_shell_command(command: &str, cwd: &Path) -> Result<String, ToolError> {
    let mut child = build_command(command, cwd)
        // stdin 显式置空：否则继承父进程 stdin，读 stdin 的命令（cat / sort / grep 无参等）
        // 会一直阻塞到超时。置空让它们立即拿到 EOF 正常结束。
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(map_spawn_error)?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    // 记录：reader 线程用 read_to_end 在内存中无上限累积；MAX_OUTPUT_BYTES 只截断最终
    // 格式化文本，不限制内存。120s 内疯狂输出的命令（如 `yes`）会先吃光内存才轮到截断。
    // MVP 风险中等；如需加固，可在 reader 内用 take(limit) 读到上限即止。
    let stdout_thread = stdout_pipe.map(|mut out| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = io::Read::read_to_end(&mut out, &mut buf);
            buf
        })
    });
    let stderr_thread = stderr_pipe.map(|mut err| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = io::Read::read_to_end(&mut err, &mut buf);
            buf
        })
    });

    let deadline = Instant::now() + Duration::from_secs(DEFAULT_TIMEOUT_SECS);
    let timed_out = loop {
        match child.try_wait() {
            Ok(Some(_status)) => break false,
            Ok(None) => {
                if Instant::now() >= deadline {
                    // 记录：kill 只杀 sh/cmd 本身，其派生的孙进程（后台 `&` 等）可能存活。
                    // 健壮做法是杀进程组（unix setpgid + killpg），MVP 暂不处理。
                    let _ = child.kill();
                    let _ = child.wait();
                    break true;
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "wait failed: {e}"
                )));
            }
        }
    };

    let status = if timed_out {
        None
    } else {
        Some(
            child
                .wait()
                .map_err(|e| ToolError::ExecutionFailed(format!("wait failed: {e}")))?,
        )
    };

    let stdout = stdout_thread
        .map(|h| h.join())
        .transpose()
        .map_err(|_| ToolError::ExecutionFailed("stdout reader panicked".into()))?
        .unwrap_or_default();
    let stderr = stderr_thread
        .map(|h| h.join())
        .transpose()
        .map_err(|_| ToolError::ExecutionFailed("stderr reader panicked".into()))?
        .unwrap_or_default();

    if timed_out {
        return Ok(format_timed_out(&stdout, &stderr));
    }

    let exit_code = status.and_then(|s| s.code());
    Ok(format_output(
        exit_code,
        &truncate_stream(&stdout, MAX_OUTPUT_BYTES),
        &truncate_stream(&stderr, MAX_OUTPUT_BYTES),
    ))
}

#[cfg(unix)]
fn build_command(command: &str, cwd: &Path) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(command).current_dir(cwd);
    cmd
}

#[cfg(windows)]
fn build_command(command: &str, cwd: &Path) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(command).current_dir(cwd);
    cmd
}

#[cfg(not(any(unix, windows)))]
fn build_command(command: &str, cwd: &Path) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(command).current_dir(cwd);
    cmd
}

fn format_timed_out(stdout: &[u8], stderr: &[u8]) -> String {
    format!(
        "exit_code: null\ntimed_out: true\nstdout:\n{}\nstderr:\n{}",
        truncate_stream(stdout, MAX_OUTPUT_BYTES),
        truncate_stream(stderr, MAX_OUTPUT_BYTES),
    )
}

fn truncate_stream(bytes: &[u8], limit: usize) -> String {
    if bytes.len() <= limit {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut text = String::from_utf8_lossy(&bytes[..limit]).into_owned();
    text.push_str(&format!("\n[truncated at {limit} bytes]"));
    text
}

fn format_output(exit_code: Option<i32>, stdout: &str, stderr: &str) -> String {
    let code_line = match exit_code {
        Some(code) => format!("exit_code: {code}"),
        None => "exit_code: null".into(),
    };
    format!("{code_line}\nstdout:\n{stdout}\nstderr:\n{stderr}")
}

fn map_spawn_error(err: io::Error) -> ToolError {
    ToolError::ExecutionFailed(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_workspace() -> (FsConfig, std::path::PathBuf) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("strata_shell_test_{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");
        let config = FsConfig::new(&dir).expect("config");
        (config, dir)
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_has_run_command_name() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("strata_shell_schema_{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");
        let config = FsConfig::new(&dir).expect("config");
        let schema = RunCommand::new(config).schema();
        assert_eq!(schema.name, "run_command");
        assert_eq!(schema.parameters["required"], serde_json::json!(["command"]));
        cleanup(&dir);
    }

    #[test]
    fn empty_command_rejected() {
        let (config, dir) = test_workspace();
        let err = RunCommand::new(config)
            .execute(serde_json::json!({ "command": "   " }))
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
        cleanup(&dir);
    }

    #[test]
    fn echo_hello() {
        let (config, dir) = test_workspace();
        let out = RunCommand::new(config)
            .execute(serde_json::json!({ "command": "echo hello" }))
            .expect("run");
        assert!(out.contains("exit_code: 0"));
        assert!(out.contains("hello"));
        cleanup(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn nonzero_exit_still_ok_unix() {
        let (config, dir) = test_workspace();
        let out = RunCommand::new(config)
            .execute(serde_json::json!({ "command": "exit 1" }))
            .expect("nonzero should be Ok");
        assert!(out.contains("exit_code: 1"));
        cleanup(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn nonzero_exit_still_ok_windows() {
        let (config, dir) = test_workspace();
        let out = RunCommand::new(config)
            .execute(serde_json::json!({ "command": "exit /b 1" }))
            .expect("nonzero should be Ok");
        assert!(out.contains("exit_code: 1"));
        cleanup(&dir);
    }

    #[test]
    fn cwd_subdir() {
        let (config, dir) = test_workspace();
        fs::create_dir(dir.join("sub")).expect("mkdir");

        #[cfg(unix)]
        let command = "pwd";
        #[cfg(windows)]
        let command = "cd";

        let out = RunCommand::new(config)
            .execute(serde_json::json!({ "command": command, "cwd": "sub" }))
            .expect("run");
        assert!(out.contains("exit_code: 0"));
        cleanup(&dir);
    }

    #[test]
    fn cwd_escape_rejected() {
        let (config, dir) = test_workspace();
        let err = RunCommand::new(config)
            .execute(serde_json::json!({
                "command": "echo x",
                "cwd": "../outside"
            }))
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
        cleanup(&dir);
    }

    #[test]
    fn cwd_not_directory() {
        let (config, dir) = test_workspace();
        fs::write(dir.join("file.txt"), "x").expect("write");
        let err = RunCommand::new(config)
            .execute(serde_json::json!({
                "command": "echo x",
                "cwd": "file.txt"
            }))
            .unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
        cleanup(&dir);
    }

    #[test]
    fn register_in_registry() {
        let (config, dir) = test_workspace();
        let mut registry = crate::ToolRegistry::new();
        registry.register(Box::new(RunCommand::new(config)));

        assert!(registry.get("run_command").is_some());
        let out = registry
            .get("run_command")
            .unwrap()
            .execute(serde_json::json!({ "command": "echo ok" }))
            .expect("execute");
        assert!(out.contains("ok"));
        cleanup(&dir);
    }

    #[test]
    fn command_reading_stdin_gets_eof_and_does_not_hang() {
        // stdin 被置空，读 stdin 的命令应立即拿到 EOF 正常结束（exit 0），而非阻塞到超时。
        // 若 stdin 未隔离，本测试会挂到 DEFAULT_TIMEOUT_SECS 才结束。
        let (config, dir) = test_workspace();

        #[cfg(unix)]
        let command = "cat";
        #[cfg(windows)]
        let command = "sort";

        let out = RunCommand::new(config)
            .execute(serde_json::json!({ "command": command }))
            .expect("run");
        assert!(out.contains("exit_code: 0"));
        cleanup(&dir);
    }

    #[test]
    fn truncate_stream_adds_notice() {
        let big = vec![b'x'; MAX_OUTPUT_BYTES + 10];
        let text = truncate_stream(&big, MAX_OUTPUT_BYTES);
        assert!(text.contains("[truncated at"));
        assert!(text.len() <= MAX_OUTPUT_BYTES + 64);
    }
}
