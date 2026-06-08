#!/usr/bin/env node
/*
 * preToolUse 守卫：编辑写入前比对设计不变量，违规则 deny。
 * 硬阻断的是"可机械判定"的越界（依赖倒置 / MVP 范围 / 模型分支 / 越界目录）。
 * 语义类违规（错误上抛、history 分叉等）grep 不出来，靠 rules + 人工 review。
 *
 * 调试：设环境变量 STRATA_HOOK_DEBUG=1 会把真实输入 JSON 写到
 *       .cursor/hooks/last-input.json，用于核对字段名。
 */
const fs = require("fs");
const path = require("path");

function readStdin() {
  try {
    return fs.readFileSync(0, "utf8");
  } catch {
    return "";
  }
}

function allow() {
  process.stdout.write(JSON.stringify({ permission: "allow" }));
  process.exit(0);
}

function deny(violations) {
  process.stdout.write(
    JSON.stringify({
      permission: "deny",
      agent_message:
        "越界设计边界：\n- " +
        violations.join("\n- ") +
        "\n请回到 doc/strata-runtime-kernel-design.md 对应章节，或停下问人。",
      user_message: "Strata 守卫拦截了一次越界编辑（见 agent 消息）。",
    })
  );
  process.exit(0);
}

const raw = readStdin();
let input;
try {
  input = JSON.parse(raw);
} catch {
  // 解析失败：failClosed=false，放行避免误伤
  allow();
}

if (process.env.STRATA_HOOK_DEBUG === "1") {
  try {
    fs.writeFileSync(
      path.join(__dirname, "last-input.json"),
      JSON.stringify(input, null, 2)
    );
  } catch {
    /* ignore */
  }
}

// 兼容不同字段命名地取出目标路径与将写入的内容
const ti = input.tool_input || input.input || input.arguments || input || {};
const filePath = String(
  ti.path || ti.file_path || ti.target_notebook || ti.filePath || ""
).replace(/\\/g, "/");
const content = String(
  ti.contents || ti.new_string || ti.new_str || ti.newString || ti.text || ""
);

const isRust = filePath.endsWith(".rs");
const inProviders = filePath.includes("/src/providers/");
const violations = [];

if (isRust) {
  if (/\b(reqwest|hyper|ureq|isahc)\b/.test(content) && !inProviders) {
    violations.push("HTTP 客户端只能出现在 src/providers/（依赖倒置 §决策3）");
  }
  if (/\btokio\b|async\s+fn|\.await\b/.test(content)) {
    violations.push("MVP 禁止 async / tokio（决策6 选 blocking）");
  }
  if (/\bif\s+model\s*==|\bmatch\s+model\b/.test(content)) {
    violations.push("禁止按模型名分支判断（硬验收 §10）");
  }
}

if (/\/src\/agents\//.test(filePath) || /\/src\/graph\//.test(filePath)) {
  violations.push("禁止多 agent / 图引擎目录（§5 明确不做）");
}

if (violations.length) {
  deny(violations);
} else {
  allow();
}
