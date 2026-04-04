# Event Stream 重新设计：从字符级到 Tool Call 级

## 问题

当前 `view events` 捕获的是 shell 级别的字符输入输出：
```
Input r
Input e
Input v
Input i
Input e
Input w
```

这没有语义价值。用户想看的是 agent 的**完整操作事件**：
```
[14:25:13] Claude Code → Read("src/main.rs")
[14:25:15] Claude Code → Edit("src/main.rs", line 19-40, added try_multicall function)
[14:25:18] Claude Code → Bash("cargo build --release")
[14:25:22] Claude Code → Write("docs/README.md", 47 lines)
```

## 设计目标

Event stream 应该展示的是 **agent tool call 级别**的结构化事件，而不是终端字符流。

每条事件应包含���
- 时间戳
- 来源 agent（Claude Code / Codex / Gemini）
- 工具名称（Read / Edit / Write / Bash / Grep 等）
- 参数摘要（文件路径、命令、搜索模式等）
- 执行结果摘要（成功/失败���输出行数等）

## 数据源方案对比

### 方案 A: 解析 PTY 输出中的 tool call 文本（当前方案，改进版）

Claude Code 在终端中输出的 tool call 有可识别的格式：
```
⏺ Edit(file_path="/Users/bill/zellij/src/main.rs", old_string="...", new_string="...")
⏺ Bash(command="cargo build --release")
⏺ Read(file_path="/Users/bill/zellij/src/commands.rs", offset=100, limit=50)
```

改进方向：
- 当前的 PTY 解析器已经在做启发式提取，但输出的事件太碎
- 需要做 **tool call 聚合**：把分散的 PTY 输出行聚合成一个完整的 tool call 事件
- 缓冲 PTY 输出直到检测到完整的 tool call boundary

优点：不需要 provider 侧任何配合  
缺点：依赖 Claude Code 的终端输出格式，格式变化时会 break

### 方案 B: 对接 Claude Code Hooks

Claude Code 支持 hooks 机制（在 `~/.claude/settings.json` 中配置）。
可以配置 hook 在每次 tool call 时通知 ACP。

需要调研：
- Claude Code hooks 的触发点和数据格式
- 是否支持 `PreToolUse` / `PostToolUse` hook
- Hook 输出是否包含完整的 tool call 参数和结果

优点：精确、结构化、由 provider 保证  
缺点：需要用户配置 hook，增加设置复杂度

### 方案 C: 读取 Claude Code Transcript JSONL

Claude Code 会把完整的对话历史写入 JSONL 文件：
`~/.claude/projects/<project>/conversations/<id>.jsonl`

每行是一个结构化的 JSON 对象，包含完整的 tool call 信息。

ACP 可以：
1. 找到当前 Claude Code session 对应的 transcript 文件
2. tail -f 这个文件
3. 解析每行 JSON，提取 tool call 事件

优点：数据最完整、格式稳定（Claude Code 自己的持久化格式）  
缺点：需要知道 transcript 文件路径，可能涉及 Claude Code 版本兼容性

### 方案 D: MCP Server 集成

ACP 自己作为 MCP server 运行，Claude Code 作为 MCP client 连接。
tool call 通过 MCP 协议直接传递。

优点：标准协议，最干净的集成  
缺点：实现复杂度最高，需要 Claude Code 侧配置 MCP server

## 推荐路径

**短期（Phase 1）**：方案 A 改进版
- 改进 PTY 输出解析器，做 tool call 聚合
- 过滤掉字符级噪音，只输出完整的 tool call 事件
- 工作量最小，不需要外部配合

**中期（Phase 2）**：方案 C（读 Transcript JSONL）
- 实现 transcript 文件发现和 tail 跟随
- 这是数据最丰富的方案
- 可以与方案 A 共存，transcript 有的用 transcript，没有的降级到 PTY 解析

**长期（Phase 3）**：方案 B + D
- Hooks 和 MCP 集成
- 作为官方推荐的集成方式

## Event Stream 输出格式设计

### 当前（改前）
```
2026-04-04T14:25:13Z | room | write_character | Input r
2026-04-04T14:25:13Z | room | write_character | Input e
2026-04-04T14:25:14Z | room | write_character | Input v
```

### 改后
```
━━━ 14:25:13 | Claude Code | Read ━━━
  file: src/main.rs
  lines: 1-50

━━━ 14:25:15 | Claude Code | Edit ━━━
  file: src/main.rs
  operation: insert function try_multicall (line 19-40)

━━━ 14:25:18 | Claude Code | Bash ━━━
  command: cargo build --release
  status: success (4m 44s)

━━━ 14:25:22 | Claude Code | Write ━━━
  file: docs/agent-control-plane/E2E_TEST_REPORT.md
  lines: 147 (new file)

━━━ 14:25:30 | Claude Code | Grep ━━━
  pattern: "pane_snapshot"
  path: zellij-server/src/agent_control_plane.rs
  matches: 3 files
```

### JSONL 格式（code.jsonl / events.jsonl）

```json
{
  "time": "2026-04-04T14:25:15Z",
  "stream": "events",
  "kind": "tool_call",
  "provider": "claude",
  "role": "driver",
  "tool": "Edit",
  "args": {
    "file_path": "src/main.rs",
    "line_range": "19-40",
    "operation": "insert"
  },
  "result": {
    "status": "success",
    "summary": "Added function try_multicall"
  }
}
```

## 需要修改的文件

| 文件 | 改动 |
|------|------|
| `zellij-server/src/agent_control_plane.rs` | PTY 解析器改为 tool call 聚合模式 |
| `zellij-server/src/screen.rs` | 停止写入字符级 write_character 事件 |
| `src/agent_control_plane.rs` | 新增 `ToolCallEvent` 结构体 |
| `src/commands.rs` | `view events` 格式化输出 |

## 与 `view code` 重新设计的关系

这两个重新设计是互补的：

- **`view events`**：显示 agent 的操作流（tool call 列表）— "agent 做了什么"
- **`view code`**：显示代码变更的 diff 内容 — "代码变成了什么样"

数据来源可以共享同一个 tool call 解析器：
- events stream 记录所有 tool call
- code stream 只记录产生代码变更的 tool call（Edit/Write），并附带 diff 内容
