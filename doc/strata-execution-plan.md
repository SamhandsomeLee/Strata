# Strata — 执行计划（功能点 ↔ Commit 映射）

> **这份文档是什么**：把 `strata-runtime-kernel-design.md` 的里程碑（§8）和 MVP 范围（§5）拆成**功能点级别的提交序列**。**每个功能点对应一条 commit**，每条都标注设计文档依据与完成标准（Done）。
>
> **执行原则**：严格按顺序推进，一条 commit 只做一件事（见 `.cursor/rules/60-git-commit.mdc`）。每条 commit 的 message 已给出，落地时直接用。每过一个里程碑，内核都应**可运行、可演示**（呼应"先 working 再 general"）。
>
> **提交格式**：`<type>(<scope>): <subject>`，body 解释 why，footer 标 `Milestone`。scope 对应内核模块。

---

## 里程碑总览

| 里程碑 | 目标 | Commit 数 | 完成线 |
|---|---|---|---|
| M0 骨架 | 项目结构 + 统一消息模型 + 各 trait 空实现，编译通过 | C01–C09 | `cargo build` 通过 |
| M1 单循环 | 接 DeepSeek，跑通"提问→纯文本回答"（无工具） | C10–C13 | 问答循环 + tracing 可见 |
| M2 工具 | ToolRegistry + JSON backend + calculator，跑通一次工具闭环 | C14–C16 | 完整工具调用闭环 |
| M3 健壮性 | 错误回填 + max_turns + 结构化 tracing | C17–C20 | 喂错不崩、超限优雅终止 |
| M4 真场景 | fs/bash 工具，跑通一个日常真任务 ← **MVP 完成线** | C21–C24 | 真实任务全程可复盘 |
| M5+ 增量 | 第二 provider / streaming / code-action / checkpoint… | backlog | 按需逐个 |

---

## M0 — 骨架（C01–C09）

目标：纯类型与接口，不接网络。对应设计文档 §1（分层）、§2（接口契约）、§3（循环签名）。

| # | Commit message | 文档依据 | Done |
|---|---|---|---|
| C01 | `chore(project): 建立 Cargo 项目与模块布局` | §1 | `cargo new` + lib.rs 声明 message/error/provider/tool/action/session/trace/run 模块；`cargo build` 通过 |
| C02 | `feat(message): 统一消息模型 Role/Message/ContentBlock` | §2.1 | 三个类型 + serde 派生；ContentBlock 含 Text/ToolCall/ToolResult；可单测序列化 |
| C03 | `feat(error): 错误分类枚举 StrataError` | §6 | Provider/Loop 错误用 thiserror；明确 parse/tool 错误不进 Err（走回填） |
| C04 | `feat(provider): Provider trait 与 Completion 请求/响应类型` | §2.2 | trait + CompletionRequest/Response + ProviderError；签名不泄漏 provider 特有概念 |
| C05 | `feat(tool): Tool trait 与 ToolRegistry` | §2.3 | trait(schema/execute) + ToolSchema + ToolError + 注册表 name→Box<dyn Tool> |
| C06 | `feat(action): ActionBackend trait 与 Action 类型` | §2.3 | trait(parse_actions) + Action 结构；JsonToolCall 留空实现 |
| C07 | `feat(session): Session 状态结构` | §2.4 | history: Vec<Message> + turn；预留可序列化边界 |
| C08 | `feat(trace): Tracer trait 与 TraceEvent` | §2.4 | trait + 事件枚举(turn_start/provider_call/tool_call/tool_result/turn_end/error) + ConsoleTracer |
| C09 | `feat(run): agentic loop 骨架（控制流 + todo 占位）` | §3 | run() 签名完整、控制流在位、内部用 todo!()；`cargo build` + `cargo clippy` 干净 |

**M0 验收**：编译通过；应用层无任何 provider 特有逻辑；trait 边界可 review。

---

## M1 — 单循环（C10–C13）

目标：接真实 DeepSeek，跑通无工具的问答循环。对应 §2.2、§3、§5。

> ⚠️ **C10 需引入 HTTP 客户端**：此时把 `reqwest` 从 `.cursor/hooks/guard-deps.js` 的 banned 列表移出（仅 reqwest，tokio 等仍禁），并确认它只出现在 `src/providers/`。

| # | Commit message | 文档依据 | Done |
|---|---|---|---|
| C10 | `feat(provider): DeepSeek provider 实现（无工具路径）` | §2.2 | OpenAI 兼容格式；鉴权(env 读 key)、序列化、解析响应→统一 Message；reqwest::blocking |
| C11 | `feat(action): JsonToolCall backend 落地 parse_actions` | §2.3 | 从 ContentBlock::ToolCall 提取 Action；无 tool call 返回空 |
| C12 | `feat(run): 落地单循环纯文本终止路径` | §3 | call→parse→空 action 即返回最终文本；每轮发 trace 事件 |
| C13 | `feat(examples): ask demo 跑通单模型问答` | §8 M1 | examples/ask.rs：输入问题→DeepSeek→纯文本答案；tracing 能看到 provider 调用 |

**M1 验收**：单模型问答循环跑通，无工具，tracing 可见 provider 调用。

---

## M2 — 工具（C14–C16）

目标：跑通一次完整工具调用闭环。对应 §3、§5。

| # | Commit message | 文档依据 | Done |
|---|---|---|---|
| C14 | `feat(tool): calculator 工具实现` | §5 | 实现 Tool trait；schema 喂模型；execute 算术求值 |
| C15 | `feat(run): 工具执行闭环（执行→回填→继续）` | §3 | 有 action 时执行每个 tool，结果回填为 ToolResult 消息，带着结果进下一轮 |
| C16 | `feat(examples): 工具调用 demo` | §8 M2 | demo 触发一次 calculator 调用并得到最终答案；闭环可复盘 |

**M2 验收**：模型发起 tool call → 解析 → 执行 → 回填 → 继续 → 最终答案。

---

## M3 — 健壮性（C17–C20）

目标：错误不崩、失控有兜底、可观测完整。对应 §3、§6。这是"可靠最小内核"成型点。

| # | Commit message | 文档依据 | Done |
|---|---|---|---|
| C17 | `feat(run): max_turns 兜底与失控保护` | §3 | 超 max_turns 返回 LoopError 并带部分结果，不无限循环 |
| C18 | `feat(run): parse/tool 错误回填不崩溃` | §3,§6 | 非法 JSON / 工具报错 / 工具不存在 → 回填 is_error 结果让模型纠正，绝不 panic/上抛 |
| C19 | `feat(trace): 结构化事件流补全（token/耗时/错误）` | §6 | 每轮 provider 调用、token、工具调用与结果、耗时、错误结构化输出 |
| C20 | `test(run): MockProvider 与循环语义测试` | §3,§8 M3 | MockProvider 脚本化返回；测终止/工具闭环/错误回填/max_turns，不联网 |

**M3 验收**：故意喂非法 JSON / 让工具报错，内核不崩、能回填纠正；超 max_turns 优雅终止。

---

## M4 — 真场景（C21–C24）← MVP 完成线

目标：用 Strata 完成一件你今天真会手动做的事。对应 §5、§8 M4。

| # | Commit message | 文档依据 | Done |
|---|---|---|---|
| C21 | `feat(tool): 文件读写工具（fs）` | §5 | 读文件 / 写文件 / 列目录；路径与错误安全处理 |
| C22 | `feat(tool): 命令执行工具（bash/shell）` | §5 | 执行命令并捕获 stdout/stderr/exit code，回填为结果 |
| C23 | `feat(examples): 跑通一个日常真任务` | §8 M4 | 例：读某目录代码并改一处，或整理一批文件；全程 tracing 可复盘 |
| C24 | `docs(doc): 记录 MVP 验收结果` | §10 | 勾选 §10 验收清单；确认应用层无任何 provider 特有逻辑 |

**M4 验收（= MVP 完成）**：真实任务全程跑通且可复盘；硬验收——应用层零 provider 特有逻辑。

---

## M5+ — 增量 Backlog（按需，逐个，不提前做）

每项独立成若干 commit，**出现真实需求再启动**（§5 "明确先不做" + §7 扩展点）。

| 候选 | 触发条件 | 涉及扩展点 |
|---|---|---|
| 第二个 provider | 真要接第二个模型时（届时适配层形态自然浮现） | §决策3、§7 Provider |
| streaming + 可中断 steering | 确认要 CC 那种交互体验时（升级 async） | §决策6 |
| CodeAction backend | 需要 code-as-action（需沙箱） | §决策2、§7 ActionBackend |
| Session checkpoint/resume | 长任务需恢复 | §决策5、§7 序列化边界 |
| 上下文注入 / 项目记忆 | 类 CLAUDE.md 需求 | §决策4 |
| permission gate | 工具执行前需确认 | §3 注释 |
| MCP 工具接入 / subagent | 需要外部工具或嵌套循环 | §5 第二层 |

> **反模式自查（每条 commit 前过一遍 §9）**：先做通用层？一上来多 agent？model 差异泄漏应用层？图引擎冲动？async 过早？tracing 拖到后面？命中任一即停。

---

## 进度追踪

- [ ] M0 骨架（C01–C09）
- [ ] M1 单循环（C10–C13）
- [ ] M2 工具（C14–C16）
- [ ] M3 健壮性（C17–C20）
- [ ] M4 真场景（C21–C24）← MVP 完成
