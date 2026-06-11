# Strata 实现宪法

唯一权威是 `doc/strata-runtime-kernel-design.md`。实现任何东西前，先在脑中对应到它的章节；
本文件与设计文档冲突时，以设计文档为准。

## 六条不可违背原则（哲学 §0）

1. 单线程 `while(has_tool_call)` 循环，不是图、不是多 agent
2. 先 working 再 general（单模型单场景先跑通再抽象）
3. 薄抽象、千行量级、一个下午读懂
4. 模型差异全部收进 Provider 实现层
5. 可观测性内建（Tracer 从第一天就在循环里）
6. 应用不依赖模型能力上限

## MVP 明确不做（§5，违反即越界）

- ❌ async / tokio（决策 6 选 blocking）
- ❌ 多 agent 编排 / 角色分工
- ❌ 图引擎 / 状态机 DSL
- ❌ 第二个 provider（只有一个时不抽象多 provider）
- ❌ 复杂 planning（TODO 树 / 反思链）
- ❌ CodeAction 沙箱（MVP 只做 JsonToolCall）

## 技术栈与并发模型

- 语言：Rust
- 并发：同步 blocking（决策 6），HTTP 客户端只能出现在 `src/providers/`
- 首个 provider：DeepSeek（OpenAI 兼容格式）

## 当前里程碑

**M3：健壮性**（C17–C20）——max_turns 优雅终止、错误回填、结构化 tracing、MockProvider 测试。

M2 已完成（C14–C16）：calculator + 工具执行闭环，`examples/calc.rs` 可演示。

M1 已完成（C10–C13）：DeepSeek 问答循环跑通，`examples/ask.rs` 可演示。

### M3 执行顺序

1. **C17** `max_turns` 兜底与失控保护
2. **C18** parse/tool 错误回填不崩溃（边界强化）
3. **C19** 结构化 trace 补全
4. **C20** MockProvider 与循环语义集成测试

环境：复制 `.env.example` → `.env`，设置 `DEEPSEEK_API_KEY`。

## 越界时

停下，说明触碰了哪条原则 / 哪条不做清单，等人确认，不要自作主张扩大范围。
