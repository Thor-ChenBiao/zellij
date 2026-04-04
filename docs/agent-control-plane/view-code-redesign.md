# `view code` 重新设计方案

## 问题

当前 `view code` 显示的是 pane viewport 快���（高频的 `pane_snapshot` 事件），用户看到的是一堆：
```
Pane terminal_1 viewport updated (21 lines)
Pane terminal_1 viewport updated (21 lines)
Pane terminal_1 viewport updated (21 lines)
```

这不是用户想要的。用户想看的是 **agent 到底改了哪些代码**。

## 用户期望

> "我不想再打开一个 IDE 去看代码。`view code` 就应该让我实时看到 agent 改了哪个文件、改了哪些行，带上完整的函数上下文。"

具体需求：

1. 当 agent 执行 Edit/Write 工具时，实时显示**文件路径 + 变更 diff**
2. 上下文不应该只显示改动的那几行，应该显示**完整函数/类**，或者可配置的上下文行数
3. 类似 `git diff` 的实时版，但跟着 agent 的操作流
4. 当前的 `pane_snapshot`（viewport 快照）和 `file_hint` 应该归入 events stream

## 重新设计后的 stream 语义

| Stream | 写入内容 | 用途 |
|--------|----------|------|
| **code** | 文件变更 diff（Edit/Write/Read 操作的实际代码内容） | 看代码变更 |
| **events** | 所有操作事件（participant、review、pane_snapshot、tool_call 等） | 看操作日志 |

### code stream 写入触发条件

只有以下情况才写入 code stream：

1. **agent 修改文件**（Edit/Write tool call 被检测到）
2. **agent 读取文件**（Read tool call，可选显示）

不写入 code stream 的：
- pane_snapshot（viewport 快照）→ 移到 events 或丢弃
- file_hint（文件线索）→ 移到 events
- 普通输入/输出事件 → 保留在 events

## code stream 输出格式设���

### 每条代码变更记录

```
━━━ Edit: src/commands.rs ━━━ 2026-04-04T14:19:22Z ━━━
@@ line 537-570 (fn start_shared_room) @@

 pub(crate) fn start_shared_room(
     mut opts: CliArgs,
     persona: AgentPersona,
     shared_id: Option<String>,
 ) {
     let shared_id = resolve_default_room_id(shared_id);
+    let current_session_name = envs::get_session_name().ok();
+    let launched_inside_target_session =
+        current_session_name.as_deref() == Some(shared_id.as_str());
     let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
     ...
 }

━━━ Write: src/new_file.rs ━━━ 2026-04-04T14:19:25Z ━━━
@@ new file (47 lines) @@

 use std::path::PathBuf;

 pub struct NewThing {
     name: String,
     value: u32,
 }
 ...
```

### 格式规则

1. **分隔线**：`━━━ <操作>: <文件路径> ━━━ <时间> ━━━`
2. **位置标记**：`@@ line <范围> (<函数/类名>) @@`
3. **diff 着色**：`+` 新增行，`-` 删除行，无前缀为上下文
4. **上下文范围**：
   - 优先显示完整函数/类（通过 `{` `}` 匹配或语言语法）
   - 如果无法确定函数边界，默认显示变更行 ± 10 行上下文
   - 可通过参数控制：`zellij view code <id> --context 20`

## 实现方案

### Phase 1: 从 PTY 输出启发式提取（最小改动）

当前 server 端已经在做 PTY 输出的启发式解析（提取 `Edit(...)`, `Write(...)` 等 tool call）。扩展这个逻辑：

1. **检测 Edit/Write tool call**
   - 从 PTY 输出中匹配 `Edit(file_path=..., old_string=..., new_string=...)`
   - 或匹配 `Write(file_path=..., content=...)`

2. **读取文件获取上下文**
   - 拿到 file_path 后，读取文件内容
   - 根据变更行号，向上向下扩展到完整函数
   - 生成 diff 格式文本

3. **写入 code stream**
   - 结构化 payload 包含：file_path, operation, line_range, diff_text, function_name
   - 人类可读的 text 写入 code.log
   - 机器可读的 JSON 写入 code.jsonl

### Phase 2: 基于实际文件 diff（更准确）

1. **监控工作目录文件变化**
   - 使用 `notify`/`inotify` 监控 repo 目录
   - 当文件变化时，与内存中的上一版本做 diff

2. **关联到 agent 操作**
   - 文件变化发生在 tool call 之后，可以关联
   - 生成准确的 diff

3. **函数级上下文**
   - 使用 tree-sitter 解析（如果可用）
   - 或简单的缩进/括号匹配

### Phase 3: 支持 `git diff` 模式

增加选项 `zellij view code <id> --git`：
- 直接调用 `git diff` 获取自 room 创建以来的所有文件变更
- 适合 review 前快速浏览全貌

## 需要修改的文��

| 文件 | 改动 |
|------|------|
| `zellij-server/src/agent_control_plane.rs` | 改 `pane_snapshot` 写入目标从 Code → Events；新增 `code_change` 事件类型 |
| `zellij-server/src/screen.rs` | 在 tool call 检测到 Edit/Write 时，读取文件并生成 diff |
| `src/agent_control_plane.rs` | 新增 `CodeChangeEvent` 结构体 |
| `src/commands.rs` | `view code` 的 follow_stream 增加格式化输出，支持 `--context` 参数 |
| `zellij-utils/src/cli.rs` | `ViewCli::Code` 增加 `--context` 和 `--git` 选项 |

## `view code` 输出对比

### ��前（改前）

```
2026-04-04T14:19:22Z | room | pane_snapshot | Pane terminal_1 viewport updated (21 lines)
2026-04-04T14:19:22Z | room | pane_snapshot | Pane terminal_1 viewport updated (21 lines)
2026-04-04T14:19:23Z | room | pane_snapshot | Pane terminal_1 viewport updated (21 lines)
```

### 改后

```
━━━ Edit: src/main.rs ━━━ 2026-04-04T14:19:22Z ━━━
@@ line 19-40 (fn try_multicall) @@

+fn try_multicall() -> bool {
+    let exe = std::env::current_exe().ok();
+    let name = exe
+        .as_ref()
+        .and_then(|p| p.file_name())
+        .and_then(|n| n.to_str())
+        .unwrap_or("");
+    let persona = match name {
+        "claude" => Some(AgentPersona::Claude),
+        "codex" => Some(AgentPersona::Codex),
+        "gemini" => Some(AgentPersona::Gemini),
+        "reviewer" => Some(AgentPersona::Reviewer),
+        _ => None,
+    };
+    ...
+}

━━━ Edit: zellij-utils/assets/config/default.kdl ━━━ 2026-04-04T14:19:25Z ━━━
@@ line 235 @@

     compact-bar location="zellij:compact-bar"
+    acp-bar location="zellij:acp-bar"
     session-manager location="zellij:session-manager"
```

## 优先级

建议按 Phase 1 → Phase 2 → Phase 3 顺序实现。

Phase 1 改动最小，只需要：
1. 把 `pane_snapshot` 的写入目标从 `ViewStreamKind::Code` 改成 `ViewStreamKind::Events`
2. 在现有的 tool call 启发式检测中，对 Edit/Write 操作读取文件并写入 code stream
3. `view code` 输出加格式化

Phase 1 预估工作量：1-2 个 session。
