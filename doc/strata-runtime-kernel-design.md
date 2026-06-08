# Strata — Agent Runtime 内核设计方法

> **这份文档是什么**：一份**设计方法论**，不是实现代码。它定义 Strata 这个最小 agent runtime 内核的设计哲学、分层架构、核心接口契约、关键决策与取舍，以及落地的里程碑。目标是让你（和未来的你）在写任何一行实现前，先把"为什么这样设计"想清楚。
>
> **项目定位**：一个**最小、model-agnostic、可嵌入日常工作流**的 agent runtime 内核，在其之上构建个人应用（类 Claude Code 的编码 agent，或其他事务处理 agent）。应用层**不绑定**任何单一模型能力——DeepSeek / Claude / GPT 等当前模型能力已足以支撑日常任务。
>
> **核心约束**：本设计源自对主流内核（Claude Code nO loop、OpenAI Agents SDK、Pydantic AI、smolagents、LangGraph、Rig）的横向调研，取其设计决策之长。文中标注了每个决策"借自哪家"。

---

## 0. 设计哲学（不可违背的原则）

这六条是地基，后面所有决策都从它们推导。违背其中任何一条，就应该停下来重新想。

1. **Radical simplicity（单循环做好一件事）**。核心是一个单线程 `while(has_tool_call)` 循环，不是图、不是多 agent swarm。*借自 Claude Code 的 nO loop——它刻意维持单主线程、单条扁平消息历史，避开多 agent 的不可预测性。*
2. **先 working 再 general**。先在**单个模型 + 单个真实场景**上跑通，再抽象。先做通用层是 agent 项目最常见的过度设计陷阱。
3. **薄抽象、可 hack**。内核核心逻辑控制在**千行量级**，任何人能在一个下午读懂。*借自 smolagents（核心约 1000 行）。*
4. **模型差异收进一层**。所有 provider 的差异（API 格式、tool call 协议、token 计数）封进一个 trait 的不同实现，应用层只见统一接口。*借自 Rig 的 CompletionModel trait。*
5. **可观测性是内建，不是事后**。结构化 tracing 从第一天就在循环里，不是 v2 功能。*借自 OpenAI Agents SDK（tracing 默认开）。*
6. **应用不依赖模型能力的上限**。内核为"最弱可接受模型"设计行为契约；强模型表现更好是 bonus，不是前提。

---

## 1. 架构总览（分层 + 依赖方向）

```
┌─────────────────────────────────────────────┐
│  Application 层（在内核之上，非内核一部分）      │
│  - 编码 agent / 事务 agent / CLI / TUI         │
└───────────────────────┬─────────────────────┘
                        │ 只依赖内核的公开接口
┌───────────────────────▼─────────────────────┐
│  Strata Kernel                               │
│                                              │
│  ┌────────────────────────────────────────┐ │
│  │  Agentic Loop（控制核心）                 │ │
│  │  while(has_tool_call): call→parse→exec   │ │
│  └──────┬─────────────┬──────────┬─────────┘ │
│         │             │          │           │
│  ┌──────▼─────┐ ┌─────▼────┐ ┌───▼────────┐  │
│  │ Provider   │ │ Action/  │ │ Context/   │  │
│  │ 抽象层      │ │ Tool 层  │ │ Session    │  │
│  └────────────┘ └──────────┘ └────────────┘  │
│         └─────────────┴──────────┘           │
│              Observability（贯穿全层）          │
└──────────────────────────────────────────────┘
         │ provider impl    │ tool impl
   ┌─────▼─────┐      ┌──────▼──────┐
   │ OpenAI /  │      │ calculator/ │
   │ Anthropic/│      │ fs / bash / │
   │ DeepSeek  │      │ ...         │
   └───────────┘      └─────────────┘
```

**依赖倒置是核心**：内核定义接口（trait），具体 provider 和 tool 是接口的实现，从外部注入。内核**不依赖**任何具体模型或工具。这是 model-agnostic 的结构性保证——不是靠 if/else 判断模型，而是靠接口隔离。

---

## 2. 核心抽象（接口契约）

下面用 Rust trait 表达（贴合你的技术栈），但**设计是语言无关的**——换成 Python ABC / TS interface 同理。关键处留 TODO，由你实现。

### 2.1 统一消息模型

所有 provider 的输入输出归一到同一组类型。这是 model-agnostic 的数据基础。

```rust
pub enum Role { System, User, Assistant, Tool }

pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,   // 文本 / 工具调用 / 工具结果，统一表示
}

pub enum ContentBlock {
    Text(String),
    ToolCall { id: String, name: String, args: serde_json::Value },
    ToolResult { id: String, content: String, is_error: bool },
}
// TODO: 不同 provider 的原生格式(OpenAI function call / Anthropic tool_use / DSML)
//       在各自 Provider 实现里翻译成/出这套统一表示，应用层永远只见这套。
```

### 2.2 Provider 抽象（model-agnostic 的核心）

```rust
pub trait Provider {
    /// 输入统一 messages + 可用工具声明，输出统一的 assistant 消息
    /// （可能含 text 和/或 tool_calls）。
    fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError>;
    // TODO: 各 impl 负责：鉴权、序列化成该 provider 的 API 格式、
    //       解析响应、把原生 tool call 格式翻译回 ContentBlock::ToolCall
}

pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,        // 工具声明，provider 各自转成自家格式
    pub max_tokens: u32,
    // temperature 等
}
```

> **设计要点**：`complete` 的签名**不暴露任何 provider 特有概念**。OpenAI 的 function calling、Anthropic 的 tool_use、DeepSeek 的 DSML 差异，全部死在各自 impl 内部。你研究过 DSML，这层你比一般人更有资格做对。

### 2.3 Tool / Action 抽象（留 backend 可替换口子）

```rust
pub trait Tool {
    fn schema(&self) -> ToolSchema;                     // name + 参数描述，喂给模型
    fn execute(&self, args: serde_json::Value) -> Result<String, ToolError>;
}

pub struct ToolRegistry { /* name -> Box<dyn Tool> */ }

/// Action backend：如何把"模型的意图"变成"可执行的调用"。
/// MVP 只实现 JsonToolCall；给 CodeAction 留 trait 口子（见决策表第 2 条）。
pub trait ActionBackend {
    fn parse_actions(&self, assistant_msg: &Message) -> Vec<Action>;
    // JsonToolCall: 从 ContentBlock::ToolCall 提取
    // CodeAction(未来): 从代码块提取并准备沙箱执行
}
```

### 2.4 Loop 状态与可观测性

```rust
pub struct Session {
    pub history: Vec<Message>,         // 单条扁平历史（借 CC）
    pub turn: u32,
    // TODO: 可序列化，支持 checkpoint/resume（借 LangGraph 思想，但无图引擎）
}

pub trait Tracer {
    fn on_event(&self, ev: TraceEvent);  // turn_start / provider_call / tool_call / tool_result / turn_end / error
}
```

---

## 3. Agentic Loop 的精确语义

内核的心脏。一个状态机，语义必须精确——这是 agent 行为可预测的来源。

```rust
fn run(session: &mut Session, provider: &dyn Provider, tools: &ToolRegistry,
       backend: &dyn ActionBackend, tracer: &dyn Tracer, max_turns: u32)
       -> Result<String, LoopError> {
    loop {
        if session.turn >= max_turns { return Err(LoopError::MaxTurns); }  // 防失控
        tracer.on_event(TraceEvent::TurnStart(session.turn));

        // 1) 调模型
        let resp = provider.complete(build_request(session, tools))?;
        session.history.push(resp.message.clone());

        // 2) 解析：有工具调用 or 纯文本？
        let actions = backend.parse_actions(&resp.message);
        if actions.is_empty() {
            // 纯文本 = 最终回答，循环自然终止（借 CC：无 tool call 即结束）
            tracer.on_event(TraceEvent::TurnEnd);
            return Ok(resp.message.text());
        }

        // 3) 执行每个工具调用，结果回填为 ToolResult 消息
        for action in actions {
            // TODO: permission gate（借 CC：执行前可确认）—— MVP 可先放行
            let result = match tools.get(&action.name) {
                Some(tool) => tool.execute(action.args),
                None => Err(ToolError::Unknown(action.name.clone())),
            };
            // 关键容错：执行失败不 crash，把错误回填让模型自我纠正
            let block = match result {
                Ok(out)  => ContentBlock::ToolResult { id: action.id, content: out, is_error: false },
                Err(e)   => ContentBlock::ToolResult { id: action.id, content: e.to_string(), is_error: true },
            };
            session.history.push(Message::tool(block));
            tracer.on_event(TraceEvent::ToolResult { /* ... */ });
        }
        session.turn += 1;
        // 4) 带着工具结果继续下一轮
    }
}
```

**语义要点**：
- **终止条件**：模型输出纯文本（无 action）→ 返回最终答案。这是唯一的正常终止。
- **失控保护**：`max_turns` 兜底，防止无限工具调用循环。
- **错误不中断循环**：工具执行失败、参数非法、工具不存在——都**回填成 tool_result（is_error）**让模型自我纠正，而不是抛异常退出。*这是真实 harness 最花功夫的地方，你在 124M 的 harness 里已经趟过。*
- **单条扁平 history**：所有 turn 共享一条消息历史，不分叉。

---

## 4. 关键设计决策（决策 / 选择 / 理由 / 代价 / 来源）

| # | 决策点 | 选择 | 理由 | 代价 | 借自 |
|---|---|---|---|---|---|
| 1 | 控制流模型 | 单线程 while 循环 | 简单、可调试、模型驱动 | 不适合复杂确定性工作流 | CC nO |
| 2 | action 表示 | JSON 先行，backend 可换 | 小/中模型可靠、易解析 | code-action 的组合性暂缺 | smolagents 留口子 |
| 3 | provider | trait 抽象 + 统一消息 | model-agnostic 的结构保证 | 每加一个 provider 要写 impl | Rig |
| 4 | 上下文 | 单条扁平 history + 可注入 | 简单、可预测 | 长任务需手动压缩 | CC（CLAUDE.md 注入） |
| 5 | 状态持久化 | Session 可序列化、checkpoint/resume | 长任务可恢复 | 需定义可序列化边界 | LangGraph 思想（无图） |
| 6 | 并发模型 | **见下方专门讨论** | 取决于是否要 streaming/steering | — | — |
| 7 | 多 agent | **不做** | 过度设计，违背原则 1/2 | 复杂分工场景需另想 | （反 CrewAI/AutoGen） |

### 决策 6 专题：同步 vs 异步（你的 open item）

这是影响整个并发模型的决策，必须早定：

- **同步（blocking，如 `reqwest::blocking`）**：最简单，顺序执行 call→parse→exec→loop，契合你 embedded 背景对确定性控制流的偏好。**MVP 推荐**。
- **异步（tokio）**：一旦要 **streaming-first**（边生成边显示）或 **可中断 steering**（任务中途注入指令，CC 的 h2A 体验），就**必须** async——因为要同时处理"模型流式输出"和"用户输入"两个事件源。

**建议路径**：MVP 用 blocking 跑通完整循环；把 `Provider` trait 设计成**不泄漏同步/异步细节**（或预留 `async fn` 版本）。当你确认要 CC 那种交互体验时，再升级到 async——这是个能隔离的改动，前提是 trait 边界设计干净。**不要为了未来可能的 streaming 在 MVP 就上 async，徒增复杂度。**

---

## 5. MVP 范围（最小必要集 vs 增量层）

### MVP 必做（最小必要集）
- [ ] 统一消息模型（Message / ContentBlock）
- [ ] `Provider` trait + **单个实现**（先接你日常用的那个模型，如 DeepSeek 或 Claude）
- [ ] 单线程 while 循环（第 3 节语义），含 max_turns + 错误回填
- [ ] `Tool` trait + ToolRegistry + **2-3 个真实工具**（如 calculator、读文件、bash）
- [ ] JSON action backend
- [ ] 基础结构化 tracing（turn / provider call / tool call / result）
- [ ] 跑通**一个你自己真会用的场景**

### 第二层（MVP 跑通后增量加，按需）
- 多 provider（加第二个 impl，这时适配层该长什么样会自然浮现）
- CodeAction backend（code-as-action，需沙箱）
- streaming + 可中断 steering（升级 async）
- Session checkpoint / resume
- 上下文注入与项目记忆（类 CLAUDE.md）
- permission gate（工具执行前确认）
- MCP 工具接入 / subagent（嵌套循环）

### 明确先不做（避免过度设计）
- ❌ 多 agent 编排 / 角色分工
- ❌ 图引擎 / 状态机 DSL
- ❌ 复杂 planning（TODO 树、反思链）
- ❌ 通用 provider 层（在只有一个 provider 时就抽象，是凭空设计）

---

## 6. 错误处理与可观测性

### Error taxonomy（分类决定恢复策略）
| 错误类 | 例子 | 恢复策略 |
|---|---|---|
| ProviderError | 网络、鉴权、限流 | 重试（带退避）/ 上抛 |
| ParseError | 模型输出非法 JSON | 回填错误提示，让模型重试（不 crash） |
| ToolError | 工具执行失败、参数非法、工具不存在 | 回填 is_error 结果，让模型自我纠正 |
| LoopError | 超 max_turns | 终止并返回部分结果 + 原因 |

> 原则：**provider 层的错误可以上抛；循环内的 parse/tool 错误一律回填进对话**，因为它们是模型可以纠正的。这区分是 harness 健壮性的关键。

### Tracing
- 结构化事件流（非 print 日志）：每个 turn 的 provider 调用、token、工具调用与结果、耗时、错误。
- *借 OpenAI SDK 的"默认开启"理念——它让调试开箱即用。* 你蓝图里本来就有 tracing strategy，这里把它落到 `Tracer` trait。

---

## 7. 扩展点（为未来留口子，现在只定接口不实现）

为了第二层能干净接入，MVP 阶段就把这几个 trait 边界留好：

- `ActionBackend`：JSON ↔ code 可替换（决策 2）
- `Tracer`：可换 console / 文件 / OpenTelemetry
- `Provider`：可插任意模型
- Hook 点（可选，借 swiftide 的 lifecycle hooks）：on_turn_start / on_tool_call 等，给中间件留位置
- Session 可序列化边界（决策 5）：checkpoint 的前提

---

## 8. 里程碑（落地路线）

```
M0 骨架      : 项目结构 + 统一消息模型 + Provider trait（空实现编译通过）
M1 单循环    : 接一个真实 provider，while 循环跑通"提问→纯文本回答"（无工具）
M2 工具      : 加 ToolRegistry + JSON backend + 1 个工具（calculator），跑通一次工具调用闭环
M3 健壮性    : 错误回填、max_turns、结构化 tracing —— 此时是一个可靠的最小内核
M4 真场景    : 加 2-3 个实用工具（fs/bash），跑通一个你日常真会用的任务 ← MVP 完成线
────────────────────────────────────────────────
M5+ 增量     : 第二 provider / streaming+steering / code-action / checkpoint / 记忆注入（按需，逐个）
```

**M0–M4 是"最小内核"的全部**。每过一个 M，内核都应该是**可运行、可演示**的（呼应"先 working 再 general"）。

---

## 9. 反模式（写代码时反复自查）

- **先做通用层**：只有一个 provider 时就抽象多 provider → 凭空设计。等第二个出现再抽。
- **一上来多 agent**：违背原则 1。单循环能做的别拆 agent。
- **model 差异泄漏到应用层**：应用里出现 `if model == "..."` → provider 抽象失败了。
- **图引擎冲动**：想用状态机 DSL 管控制流 → 你不需要 LangGraph，while 循环够了。
- **async 过早**：MVP 不要 streaming 就别上 tokio。
- **tracing 拖到后面**：调试痛苦会逼你回头补，不如第一天就内建。

---

## 10. 验收标准（每个里程碑的 done）

- [ ] **M1**：单模型问答循环跑通，无工具，tracing 能看到 provider 调用
- [ ] **M2**：完整工具闭环——模型发起 tool call → 解析 → 执行 → 回填 → 继续 → 最终答案
- [ ] **M3**：故意喂非法 JSON / 让工具报错，内核**不崩溃**、能回填让模型纠正；超 max_turns 优雅终止
- [ ] **M4**：用 Strata 完成一件你**今天真的会手动做**的事（例：让它读某目录代码并改一处、或整理一批文件），全程 tracing 可复盘
- [ ] 应用层代码里**没有任何 provider 特有逻辑**（model-agnostic 的硬验收）

---

## 附：这个内核为什么值得做（设计信心锚点）

调研里一个数据：harness/runtime 设计能让**同一个模型**在相同任务上的 agent 性能相差最多 30 个百分点（HAL benchmark 上 Claude Opus 4 在不同 scaffold 下 GAIA 得分 64.9% vs 57.6%）。**runtime 内核不是次要实现细节，它本身就决定 agent 好不好用。** 你选的这块，价值是结构性的。

---

**下一步**：从 M0 开始——先定项目结构和统一消息模型（第 2.1 节），这是后面所有东西的地基。写出来后发我 review `Provider` trait 的边界设计，那是 model-agnostic 成败的关键。

> 说明：本文档的设计决策来自公开调研与各家文档/源码分析，是工程方法建议而非唯一解。CC 的 nO/h2A 等内部机制来自第三方源码分析、非官方文档，细节可能有出入。所有 trait 签名是设计骨架，落地时按你的语言和实际需求调整——**关键是接口边界要守住依赖倒置和 model-agnostic 这两条**。
