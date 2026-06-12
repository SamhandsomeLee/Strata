//! Workspace-scoped filesystem tools (design §5, C21).
//!
//! All paths are relative to a configured root; traversal outside the root is rejected.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::ToolError;
use crate::tool::{Tool, ToolSchema};

/// Maximum bytes returned by [`ReadFile`]; larger files are truncated with a notice.
pub const MAX_READ_BYTES: usize = 512 * 1024;

/// Canonical workspace root for path resolution.
#[derive(Debug, Clone)]
pub struct FsConfig {
    root: PathBuf,
}

impl FsConfig {
    /// Root must exist and be a directory. Stored path is canonicalized.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, ToolError> {
        let root = root.as_ref();
        if !root.exists() {
            return Err(ToolError::ExecutionFailed(format!(
                "workspace root does not exist: {}",
                root.display()
            )));
        }
        let canonical = root.canonicalize().map_err(map_io_error)?;
        if !canonical.is_dir() {
            return Err(ToolError::ExecutionFailed(format!(
                "workspace root is not a directory: {}",
                canonical.display()
            )));
        }
        Ok(Self { root: canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Canonicalize an existing path and reject it if its real location escapes the root.
    /// Used for the parent dir on writes, where the target itself may not exist yet.
    fn ensure_within_root(&self, path: &Path) -> Result<(), ToolError> {
        let canonical = path.canonicalize().map_err(map_io_error)?;
        if !canonical.starts_with(&self.root) {
            return Err(ToolError::InvalidArgs("path outside workspace".into()));
        }
        Ok(())
    }

    fn parse_relative_path(raw: &str) -> Result<PathBuf, ToolError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(ToolError::InvalidArgs("empty path".into()));
        }
        let path = Path::new(raw);
        if path.is_absolute() {
            return Err(ToolError::InvalidArgs(
                "path must be relative to workspace root".into(),
            ));
        }
        Ok(path.to_path_buf())
    }

    /// Resolve a relative path under the workspace root.
    fn resolve(&self, raw: &str) -> Result<PathBuf, ToolError> {
        let relative = Self::parse_relative_path(raw)?;
        let mut candidate = self.root.clone();

        for component in relative.components() {
            match component {
                Component::Normal(part) => candidate.push(part),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !candidate.pop() || !candidate.starts_with(&self.root) {
                        return Err(ToolError::InvalidArgs(
                            "path outside workspace".into(),
                        ));
                    }
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolError::InvalidArgs("invalid path".into()));
                }
            }
        }

        if !candidate.starts_with(&self.root) {
            return Err(ToolError::InvalidArgs("path outside workspace".into()));
        }

        if candidate.exists() {
            let canonical = candidate.canonicalize().map_err(map_io_error)?;
            if !canonical.starts_with(&self.root) {
                return Err(ToolError::InvalidArgs("path outside workspace".into()));
            }
            return Ok(canonical);
        }

        Ok(candidate)
    }
}

fn map_io_error(err: io::Error) -> ToolError {
    ToolError::ExecutionFailed(err.to_string())
}

/// Largest index `<= limit` that lands on a UTF-8 char boundary in `bytes`.
/// Continuation bytes match `0b10xxxxxx`; we walk back off any partial sequence.
fn utf8_boundary_floor(bytes: &[u8], limit: usize) -> usize {
    let mut end = limit.min(bytes.len());
    while end > 0 && bytes.get(end).is_some_and(|b| (b & 0b1100_0000) == 0b1000_0000) {
        end -= 1;
    }
    end
}

fn require_path_arg(args: &serde_json::Value) -> Result<&str, ToolError> {
    args.get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing required field: path".into()))
}

/// Read UTF-8 text from a file under the workspace root.
#[derive(Debug, Clone)]
pub struct ReadFile {
    config: FsConfig,
}

impl ReadFile {
    pub fn new(config: FsConfig) -> Self {
        Self { config }
    }
}

impl Tool for ReadFile {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read_file".into(),
            description: "Read UTF-8 text from a file path relative to the workspace root.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to workspace root"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    fn execute(&self, args: serde_json::Value) -> Result<String, ToolError> {
        let path_str = require_path_arg(&args)?;
        let path = self.config.resolve(path_str)?;

        if path.is_dir() {
            return Err(ToolError::ExecutionFailed(format!(
                "is a directory: {path_str}"
            )));
        }
        if !path.is_file() {
            return Err(ToolError::ExecutionFailed(format!(
                "file not found: {path_str}"
            )));
        }

        let bytes = fs::read(&path).map_err(map_io_error)?;
        let truncated = bytes.len() > MAX_READ_BYTES;
        let slice = if truncated {
            // 截断到 MAX_READ_BYTES 后，回退到最近的 UTF-8 字符边界，避免把横跨边界的
            // 多字节字符切成半个导致合法文件被误判为非法 UTF-8。
            &bytes[..utf8_boundary_floor(&bytes, MAX_READ_BYTES)]
        } else {
            &bytes
        };

        let text = std::str::from_utf8(slice).map_err(|_| {
            ToolError::ExecutionFailed(format!("not valid UTF-8: {path_str}"))
        })?;

        if truncated {
            Ok(format!(
                "{text}\n[truncated at {MAX_READ_BYTES} bytes; file size {} bytes]",
                bytes.len()
            ))
        } else {
            Ok(text.to_string())
        }
    }
}

/// Write UTF-8 text to a file under the workspace root.
#[derive(Debug, Clone)]
pub struct WriteFile {
    config: FsConfig,
}

impl WriteFile {
    pub fn new(config: FsConfig) -> Self {
        Self { config }
    }
}

impl Tool for WriteFile {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_file".into(),
            description:
                "Write UTF-8 text to a file path relative to the workspace root (overwrites)."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to workspace root"
                    },
                    "content": {
                        "type": "string",
                        "description": "Text content to write"
                    },
                    "create_dirs": {
                        "type": "boolean",
                        "description": "If true, create parent directories. Default false."
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    fn execute(&self, args: serde_json::Value) -> Result<String, ToolError> {
        let path_str = require_path_arg(&args)?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing required field: content".into()))?;
        let create_dirs = args
            .get("create_dirs")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = self.config.resolve(path_str)?;

        if path.is_dir() {
            return Err(ToolError::ExecutionFailed(format!(
                "is a directory: {path_str}"
            )));
        }

        if let Some(parent) = path.parent() {
            if create_dirs {
                fs::create_dir_all(parent).map_err(map_io_error)?;
            } else if !parent.exists() {
                return Err(ToolError::ExecutionFailed(format!(
                    "parent directory does not exist: {}",
                    parent.display()
                )));
            }
            // resolve() 只在目标本身存在时 canonicalize；写新文件时目标不存在，故对已存在的
            // 父目录单独复查真实路径，堵住「workspace 内 symlink 父目录指向外部」的写逃逸。
            self.config.ensure_within_root(parent)?;
        }

        fs::write(&path, content.as_bytes()).map_err(map_io_error)?;
        Ok(format!(
            "written {} bytes to {path_str}",
            content.len()
        ))
    }
}

/// List entries in a directory under the workspace root (non-recursive).
#[derive(Debug, Clone)]
pub struct ListDir {
    config: FsConfig,
}

impl ListDir {
    pub fn new(config: FsConfig) -> Self {
        Self { config }
    }
}

impl Tool for ListDir {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "list_dir".into(),
            description:
                "List files and directories at a path relative to the workspace root.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path relative to workspace root; use \".\" for root"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    fn execute(&self, args: serde_json::Value) -> Result<String, ToolError> {
        let path_str = require_path_arg(&args)?;
        let path = self.config.resolve(path_str)?;

        if !path.is_dir() {
            return Err(ToolError::ExecutionFailed(format!(
                "not a directory: {path_str}"
            )));
        }

        let mut names: Vec<String> = fs::read_dir(&path)
            .map_err(map_io_error)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().ok()?.is_dir() {
                    Some(format!("{name}/"))
                } else {
                    Some(name)
                }
            })
            .collect();
        names.sort();

        Ok(names.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_workspace() -> (FsConfig, PathBuf) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("strata_fs_test_{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir");
        let config = FsConfig::new(&dir).expect("config");
        (config, dir)
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_rejects_absolute_path() {
        let (config, dir) = test_workspace();
        let err = config.resolve("/etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
        cleanup(&dir);
    }

    #[test]
    fn resolve_rejects_escape_via_dotdot() {
        let (config, dir) = test_workspace();
        let err = config.resolve("../outside.txt").unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
        cleanup(&dir);
    }

    #[test]
    fn read_file_returns_content() {
        let (config, dir) = test_workspace();
        let file = dir.join("hello.txt");
        fs::write(&file, "world").expect("write");

        let out = ReadFile::new(config)
            .execute(serde_json::json!({ "path": "hello.txt" }))
            .expect("read");
        assert_eq!(out, "world");
        cleanup(&dir);
    }

    #[test]
    fn read_file_not_found() {
        let (config, dir) = test_workspace();
        let err = ReadFile::new(config)
            .execute(serde_json::json!({ "path": "missing.txt" }))
            .unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
        cleanup(&dir);
    }

    #[test]
    fn read_file_rejects_directory() {
        let (config, dir) = test_workspace();
        fs::create_dir(dir.join("sub")).expect("mkdir");
        let err = ReadFile::new(config)
            .execute(serde_json::json!({ "path": "sub" }))
            .unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
        cleanup(&dir);
    }

    #[test]
    fn read_file_truncates_large_file() {
        let (config, dir) = test_workspace();
        let big = "x".repeat(MAX_READ_BYTES + 100);
        fs::write(dir.join("big.txt"), &big).expect("write");

        let out = ReadFile::new(config)
            .execute(serde_json::json!({ "path": "big.txt" }))
            .expect("read");
        assert!(out.contains("[truncated at"));
        assert!(out.starts_with(&"x".repeat(MAX_READ_BYTES)));
        cleanup(&dir);
    }

    #[test]
    fn read_file_truncates_on_char_boundary() {
        let (config, dir) = test_workspace();
        // 让一个 3 字节字符「中」恰好横跨 MAX_READ_BYTES 边界：前缀填满到 MAX-1，
        // 于是字符首字节落在 MAX-1、两个续字节落在 MAX 与 MAX+1。
        let mut content = "a".repeat(MAX_READ_BYTES - 1);
        content.push('中');
        content.push_str("tail");
        fs::write(dir.join("multibyte.txt"), &content).expect("write");

        let out = ReadFile::new(config)
            .execute(serde_json::json!({ "path": "multibyte.txt" }))
            .expect("read should not error on a valid UTF-8 file");
        assert!(out.contains("[truncated at"));
        // 回退到字符边界后只剩前缀的 'a'，不含被切断的「中」。
        assert!(out.starts_with(&"a".repeat(MAX_READ_BYTES - 1)));
        assert!(!out.contains('中'));
        cleanup(&dir);
    }

    #[test]
    fn write_file_overwrites() {
        let (config, dir) = test_workspace();
        WriteFile::new(config.clone())
            .execute(serde_json::json!({ "path": "out.txt", "content": "v1" }))
            .expect("write");
        WriteFile::new(config)
            .execute(serde_json::json!({ "path": "out.txt", "content": "v2" }))
            .expect("write");

        let text = fs::read_to_string(dir.join("out.txt")).expect("read");
        assert_eq!(text, "v2");
        cleanup(&dir);
    }

    #[test]
    fn write_file_create_dirs_false_fails() {
        let (config, dir) = test_workspace();
        let err = WriteFile::new(config)
            .execute(serde_json::json!({
                "path": "a/b/c.txt",
                "content": "x"
            }))
            .unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
        cleanup(&dir);
    }

    #[test]
    fn write_file_create_dirs_true() {
        let (config, dir) = test_workspace();
        WriteFile::new(config)
            .execute(serde_json::json!({
                "path": "a/b/c.txt",
                "content": "nested",
                "create_dirs": true
            }))
            .expect("write");

        let text = fs::read_to_string(dir.join("a/b/c.txt")).expect("read");
        assert_eq!(text, "nested");
        cleanup(&dir);
    }

    #[test]
    fn list_dir_lists_entries() {
        let (config, dir) = test_workspace();
        fs::write(dir.join("a.txt"), "").expect("write");
        fs::create_dir(dir.join("sub")).expect("mkdir");

        let out = ListDir::new(config)
            .execute(serde_json::json!({ "path": "." }))
            .expect("list");
        let lines: Vec<_> = out.lines().collect();
        assert!(lines.contains(&"a.txt"));
        assert!(lines.contains(&"sub/"));
        cleanup(&dir);
    }

    #[test]
    fn utf8_boundary_floor_backs_off_continuation_bytes() {
        // "a中" = [0x61, 0xE4, 0xB8, 0xAD]; 字符「中」起于索引 1。
        let bytes = "a中".as_bytes();
        assert_eq!(utf8_boundary_floor(bytes, 4), 4); // 已是结尾
        assert_eq!(utf8_boundary_floor(bytes, 3), 1); // 切在续字节 → 回退到「中」前
        assert_eq!(utf8_boundary_floor(bytes, 2), 1); // 同上
        assert_eq!(utf8_boundary_floor(bytes, 1), 1); // 恰在边界
        assert_eq!(utf8_boundary_floor(bytes, 0), 0);
    }

    fn unique_outside_dir() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("strata_fs_outside_{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir outside");
        dir
    }

    #[cfg(unix)]
    #[test]
    fn write_rejects_symlinked_parent_escape() {
        use std::os::unix::fs::symlink;
        let (config, dir) = test_workspace();
        let outside = unique_outside_dir();
        symlink(&outside, dir.join("link")).expect("symlink");

        let err = WriteFile::new(config)
            .execute(serde_json::json!({ "path": "link/evil.txt", "content": "x" }))
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
        assert!(!outside.join("evil.txt").exists());

        cleanup(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[cfg(windows)]
    #[test]
    fn write_rejects_symlinked_parent_escape() {
        use std::os::windows::fs::symlink_dir;
        let (config, dir) = test_workspace();
        let outside = unique_outside_dir();
        // Windows 创建目录符号链接需要权限/开发者模式；无权限时优雅跳过。
        if symlink_dir(&outside, dir.join("link")).is_err() {
            cleanup(&dir);
            let _ = fs::remove_dir_all(&outside);
            return;
        }

        let err = WriteFile::new(config)
            .execute(serde_json::json!({ "path": "link/evil.txt", "content": "x" }))
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
        assert!(!outside.join("evil.txt").exists());

        cleanup(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn register_all_in_registry() {
        let (config, dir) = test_workspace();
        let mut registry = crate::ToolRegistry::new();
        registry.register(Box::new(ReadFile::new(config.clone())));
        registry.register(Box::new(WriteFile::new(config.clone())));
        registry.register(Box::new(ListDir::new(config)));

        assert!(registry.get("read_file").is_some());
        assert!(registry.get("write_file").is_some());
        assert!(registry.get("list_dir").is_some());

        fs::write(dir.join("reg.txt"), "ok").expect("write");
        let out = registry
            .get("read_file")
            .unwrap()
            .execute(serde_json::json!({ "path": "reg.txt" }))
            .expect("read");
        assert_eq!(out, "ok");
        cleanup(&dir);
    }
}
