# Strata

**最小、model-agnostic 的 agent runtime 内核（Rust）**

Strata 是一个薄层 agent 运行时：单线程 `while(has_tool_call)` 循环，统一消息模型，Provider / Tool / Tracer 通过 trait 注入。应用层不绑定任何单一模型的 API 细节。

> 设计权威：`doc/strata-runtime-kernel-design.md`  
> 执行计划：`doc/strata-execution-plan.md`  
> 实现约束：`AGENTS.md`

**状态：MVP 已完成（M0–M4，C01–C24）**

---

## 快速开始

### 环境要求

- Rust 2024 edition（stable）
- DeepSeek API Key（examples 联网演示用）

### 配置

```bash
cp .env.example .env
# 编辑 .env，填入 DEEPSEEK_API_KEY
```

可选环境变量见 `.env.example`（`DEEPSEEK_API_BASE`、`DEEPSEEK_MODEL`、`DEEPSEEK_THINKING`）。

### 编译与测试

```bash
cargo build
cargo test          # 单元 + 集成测（不联网）
cargo clippy -- -D warnings
```

### 运行 Examples

所有 example 的 trace 事件输出到 **stderr**，最终答案输出到 **stdout**。

| Example | 里程碑 | 命令 | 说明 |
|---------|--------|------|------|
| **ask** | M1 | `cargo run --example ask -- "你的问题"` | 纯文本问答，无工具 |
| **calc** | M2 | `cargo run --example calc -- "用 calculator 计算 (17*23)+5"` | calculator 工具闭环 |
| **task** | M4 | `cargo run --example task` | fs + shell 真任务（版本修复） |

**M4 真任务（task）默认行为：**

1. 将 `examples/fixtures/version-mismatch/` 复制到临时 workspace（不修改仓库内 fixture）
2. Agent 读取 `README.md` 与 `app.toml`，修复 version 不一致
3. 用 `run_command` 验证，stderr 打印 `task verify: ok` 表示修复成功

```bash
# 默认任务
cargo run --example task

# 自定义任务 / workspace
cargo run --example task -- "自定义任务描述"
cargo run --example task -- --workspace path/to/dir "任务"
```

---

## 架构概览

```
Application (examples/)
        │  只依赖 strata 公开 API
        ▼
┌───────────────────────────────────┐
│  Strata Kernel                    │
│  run() → Provider / ToolRegistry  │
│        → ActionBackend / Tracer   │
│        → Session                  │
└───────────────────────────────────┘
        │
   ┌────┴────┐
   ▼         ▼
providers/  tools/
deepseek    calculator, fs, shell
```

### 核心模块

| 模块 | 路径 | 职责 |
|------|------|------|
| Message | `src/message.rs` | `Role` / `Message` / `ContentBlock` 统一消息模型 |
| Provider | `src/provider.rs` + `src/providers/` | 模型调用抽象；DeepSeek 为首个实现 |
| Tool | `src/tool.rs` + `src/tools/` | 工具 trait + 注册表 |
| Action | `src/action.rs` | `JsonToolCall` 解析 tool call |
| Run | `src/run.rs` | 单线程 agentic 循环 |
| Session | `src/session.rs` | 扁平 message history + turn 计数 |
| Trace | `src/trace.rs` | 结构化事件流 |
| Error | `src/error.rs` | Provider/Loop 上抛；Parse/Tool 回填 |

### 循环语义

1. 调用 Provider → 解析 Action
2. **无 tool call** → 返回纯文本，正常终止
3. **有 tool call** → 执行工具 → 结果回填为 `ToolResult` → 继续循环
4. `max_turns` 超限 → `LoopError::MaxTurns`（带 partial 结果）
5. parse / tool 错误 → **回填 `is_error: true`**，不 panic、不上抛

---

## 内置工具

| 工具名 | 模块 | 说明 |
|--------|------|------|
| `calculator` | `tools/calculator.rs` | 基础算术表达式求值 |
| `read_file` | `tools/fs.rs` | 读 workspace 内 UTF-8 文件（512 KiB 截断） |
| `write_file` | `tools/fs.rs` | 写 workspace 内文件 |
| `list_dir` | `tools/fs.rs` | 列目录（非递归） |
| `run_command` | `tools/shell.rs` | 在 workspace 内执行 shell 命令 |

**FsConfig workspace 沙箱：** 路径相对 root，禁止 `..` 逃逸；写文件时复查 symlink 父目录。  
**Shell 说明：** Unix 用 `sh -c`，Windows 用 `cmd /C`；cwd 受 workspace 限制，命令内容本身不受限（MVP 无沙箱）。

---

## MVP 验收清单（设计文档 §10）

| 验收项 | 状态 | 证据 |
|--------|------|------|
| **M1** 单模型问答，无工具，tracing 可见 provider 调用 | ✅ | `examples/ask.rs`；stderr `[strata] event=provider_call` |
| **M2** 完整工具闭环：call → parse → exec → 回填 → 继续 → 答案 | ✅ | `examples/calc.rs` |
| **M3** 非法 JSON / 工具报错不崩溃，能回填纠正；max_turns 优雅终止 | ✅ | `tests/loop_semantics.rs`（20 项，MockProvider，不联网） |
| **M4** 完成一件日常真会手动做的事，全程 tracing 可复盘 | ✅ | `examples/task.rs` + fixture `version-mismatch` |
| **硬验收** 应用层无 provider 特有逻辑 | ✅ | 见下文「model-agnostic 审查」 |

### M3 自动化覆盖（摘要）

`tests/loop_semantics.rs` 覆盖：

- 纯文本终止与 trace 事件
- 工具两轮闭环
- 未知工具 / 非法参数 / 空 tool id 回填
- max_turns 截停与 partial 结果
- Provider 失败 trace（含 duration）

### M4 人工验收步骤

```bash
cargo run --example task
```

预期：

1. stderr 出现 `workspace: ...` 临时目录路径
2. trace 中出现 `list_dir`、`read_file`、`write_file`、`run_command`
3. stdout 为 agent 任务总结
4. stderr 末尾 `task verify: ok (app.toml contains 0.2.0)`

> 依赖模型行为，偶发需重跑；fixture 与 verify 逻辑保证可客观检查文件是否改对。

---

## model-agnostic 硬验收

**原则：** 应用层只使用 `Provider` trait 及内核公开类型，不出现 `if model == "..."` 或 provider 特有 API 字段。

### Examples 审查

| 文件 | Provider 用法 | 分支/泄漏 |
|------|---------------|-----------|
| `examples/ask.rs` | `DeepSeekProvider::from_env()` | 无 |
| `examples/calc.rs` | 同上 | 无 |
| `examples/task.rs` | 同上 | 无 |

Examples 选用 DeepSeek 是**部署选择**（当前唯一 provider impl），不是架构耦合。替换 provider 只需换 `Box<dyn Provider>` 的具体实现，example 其余代码不变。

### 内核审查

- `src/run.rs`、`src/tool.rs`、`src/action.rs` 等核心模块 **不 import** `providers::` 或 `tools::`
- HTTP 客户端（`reqwest`）**仅**出现在 `src/providers/`
- `.cursor/hooks/guard-deps.js` 仍禁止 `tokio` 等 async 依赖

---

## MVP 明确不做（§5）

以下能力**未实现**，出现需求时再按 backlog 逐个启动：

- async / tokio / streaming
- 多 agent 编排
- 图引擎 / 状态机 DSL
- 第二个 provider（无通用 multi-provider 层）
- CodeAction 沙箱
- 复杂 planning（TODO 树 / 反思链）
- permission gate（工具执行前确认）
- Session checkpoint / resume
- MCP / subagent

---

## 项目结构

```
strata/
├── src/
│   ├── lib.rs           # 内核公开 API
│   ├── message.rs
│   ├── provider.rs
│   ├── providers/       # DeepSeek（唯一 HTTP）
│   ├── tool.rs
│   ├── tools/           # calculator, fs, shell
│   ├── action.rs
│   ├── run.rs
│   ├── session.rs
│   ├── trace.rs
│   └── error.rs
├── examples/
│   ├── ask.rs
│   ├── calc.rs
│   ├── task.rs
│   └── fixtures/version-mismatch/
├── tests/
│   └── loop_semantics.rs
├── doc/
│   ├── strata-runtime-kernel-design.md
│   └── strata-execution-plan.md
├── AGENTS.md
└── .env.example
```

---

## 里程碑与 Commit 映射（M0–M4）

| 里程碑 | 范围 | 完成线 |
|--------|------|--------|
| M0 骨架 | C01–C09 | `cargo build` |
| M1 单循环 | C10–C13 | `examples/ask.rs` |
| M2 工具 | C14–C16 | `examples/calc.rs` |
| M3 健壮性 | C17–C20 | `tests/loop_semantics.rs` |
| **M4 真场景** | **C21–C24** | **`examples/task.rs` + 本文档** |

M4 相关 commit：

```
1e5ff1e feat(tool): 文件读写工具（fs）
f4f6719 feat(tool): 命令执行工具（bash/shell）
a1b65d1 feat(examples): 跑通一个日常真任务
```

---

## 已知限制（MVP 诚实声明）

1. **单 Provider**：仅 DeepSeek；换模型需新 impl，不抽象多 provider 层。
2. **同步 blocking**：无 streaming；长任务无法中途注入指令。
3. **Shell 非沙箱**：`run_command` 可执行任意命令；仅 cwd 限制在 workspace。
4. **模型依赖**：task example 成功率取决于模型是否按 prompt 调用工具。
5. **Trace 粒度**：provider 调用有耗时/token；工具执行耗时未单独记录。

---

## 文档索引

| 文档 | 内容 |
|------|------|
| [strata-runtime-kernel-design.md](doc/strata-runtime-kernel-design.md) | 设计哲学、接口契约、决策表、验收标准 |
| [strata-execution-plan.md](doc/strata-execution-plan.md) | C01–C24 功能点与 commit 映射 |
| [AGENTS.md](AGENTS.md) | 实现宪法与 MVP 边界 |

---

## License

见仓库根目录 LICENSE 文件（若未添加则待补充）。
