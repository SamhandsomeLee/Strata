#!/usr/bin/env node
/*
 * beforeShellExecution 守卫：拦截引入违禁/越界依赖的 `cargo add`。
 * reqwest 已在 M1 准备阶段移出黑名单（仅允许通过 C10+ 加入，且只能用于 src/providers/）。
 * 返回 "ask" 而非 "deny"：留人工放行口子，避免合理依赖被一刀切。
 */
const fs = require("fs");

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

const raw = readStdin();
let input;
try {
  input = JSON.parse(raw);
} catch {
  allow();
}

const cmd = String(input.command || input.cmd || "");

const banned = ["tokio", "async-trait", "futures", "petgraph"];
const hit = banned.filter((p) =>
  new RegExp(`cargo\\s+add\\s+[^\\n]*\\b${p}\\b`).test(cmd)
);

if (hit.length) {
  process.stdout.write(
    JSON.stringify({
      permission: "ask",
      user_message: `准备引入违禁 / 越界依赖：${hit.join(", ")}。确认要加吗？`,
      agent_message: `${hit.join(
        ", "
      )} 违反 MVP 范围（§5）或千行量级原则。除非用户确认，否则不要添加。`,
    })
  );
  process.exit(0);
}

allow();
