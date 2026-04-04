# Agent Control Plane 测试报告

**日期**: 2026-04-04  
**分支**: agent-control-plane-design  
**测试环境**: macOS Darwin 24.6.0, Rust 1.94.1

---

## 一、运行时测试结果

### 1. agent_control_plane.rs 单元测试

| 测试名 | 结果 |
|--------|------|
| `extracts_tool_calls_and_file_hints_from_terminal_lines` | PASS |
| `deduplicates_identical_viewport_snapshots` | PASS |
| `trims_long_multibyte_partial_lines_without_panicking` | PASS |

**结论**: 3/3 全部通过，无失败。

### 2. status-bar 插件编译检查

| 检查项 | 结果 |
|--------|------|
| `cargo check -p status-bar` | PASS |

**结论**: 编译通过，无错误、无警告。

---

## 二、发现的问题

### 问题 1 (中) — `keybinds()` 中 ACP hint 依赖 `/tmp` 文件存在

**文件**: `default-plugins/status-bar/src/second_line.rs:425-428`

```rust
pub fn keybinds(help: &ModeInfo, tip_name: &str, max_width: usize) -> LinePart {
    if let Some(agent_control_hint) = agent_control_plane_hint(help, None, None, max_width) {
        return agent_control_hint;
    }
```

在 `keybinds()` 调用路径上，`shared_id` 和 `focused_pane` 都是 `None`。
此时 `agent_control_plane_hint` 会回退到 `help.session_name` 作为 shared_id，然后检查 `room_hint_exists()`，即 `/tmp/zellij-agent-control-plane/{session_name}.room` 是否存在。

**问题**:
- 只要 room hint 文件存在，**所有 pane** 的状态栏都会显示 ACP hint，不区分是否是 agent pane
- 此时 `focused_pane` 为 None，`is_agent_control_pane` 判定失败，但 `||` 左侧 `room_hint_exists` 为 true 就够了
- 这意味着用户在普通编辑 pane 上也会看到 ACP hint 替代了快捷键提示

**预期行为**: 应该区分：在 agent pane 上显示完整 ACP hint，在普通 pane 上保留原始 keybinds 或显示简化的 ACP 状态。

**严重程度**: 中

---

### 问题 2 (低) — `is_agent_control_pane` 匹配词过于宽泛

**文件**: `default-plugins/status-bar/src/second_line.rs:94-96`

```rust
matches!(command, "claude" | "codex" | "gemini")
    || focused_pane.title.contains("Claude Code")
    || focused_pane.title.contains("Codex")
    || focused_pane.title.contains("Gemini")
    || focused_pane.title.contains("Reviewer")
```

- `"Reviewer"` 太通用 — 如果 pane title 包含文件名 `code_reviewer.py` 就会误匹配
- `"Codex"` 同理 — 编辑 `codex.yaml` 时可能匹配

**建议**: 使用更精确的前缀/全词匹配，如 `"[Reviewer]"` 或 `"ACP Reviewer"`

**严重程度**: 低

---

### 问题 3 (信息) — 多字节截断测试断言不够精确

**文件**: `zellij-server/src/agent_control_plane.rs:279`

```rust
assert!(buffer.len() <= 1024);
```

"你" = 3 字节 UTF-8。输入 2000 个 "你" = 6000 字节 > 4096，触发截断。
`suffix_start_for_max_bytes("你"×2000, 1024)`:
- `6000 - 1024 = 4976`，4976 不是字符边界（4976 % 3 = 2），向前调至 4977（4977 % 3 = 0）
- 保留 `6000 - 4977 = 1023` 字节 = 341 个 "你"

断言 `<= 1024` 成立但不精确。建议改为：
```rust
assert_eq!(buffer.len(), 1023);
assert_eq!(buffer.chars().count(), 341);
```

**严重程度**: 信息性 — 不影响功能

---

### 问题 4 (信息) — `room_hint_exists` 硬编码 `/tmp` 路径

**文件**: `default-plugins/status-bar/src/second_line.rs:88-92`

```rust
fn room_hint_exists(session_name: &str) -> bool {
    PathBuf::from("/tmp/zellij-agent-control-plane")
        .join(format!("{}.room", session_name))
        .exists()
}
```

硬编码 `/tmp` 在 macOS 上可行，但：
- Linux 上 systemd 可能将 `/tmp` 挂载为 tmpfs 并在重启后清除（这可能是期望行为）
- 多用户环境下不同用户共享 `/tmp`，存在 session name 冲突风险
- 与 `agent_control_plane.rs` 中的 room 文件路径是否一致？需要确认服务端写入的路径也是 `/tmp/zellij-agent-control-plane/`

**严重程度**: 信息性 — 取决于设计决策

---

### 问题 5 (信息) — `truncate_middle` 的 `max_len == 0` 边界

当 `max_len == 0` 且文本非空时，返回空字符串。行为上没问题，但调用方 `max_width.saturating_sub(5)` 在 `max_width < 5` 时会产生 `max_len = 0`，建议确认这个极端窄宽度下的渲染是否美观。

**严重程度**: 信息性

---

## 三、测试覆盖度评估

### 已覆盖
- tool call 和 file hint 解析 ✓
- viewport 去重 ✓
- 多字节 UTF-8 截断安全 ✓

### 建议补充的测试
1. **ASCII 长文本截断**: 验证纯 ASCII buffer > 4096 的截断行为
2. **混合 ASCII + 多字节截断**: 混合内容的字符边界处理
3. **空输入**: `ingest_pty_bytes` 传入空 bytes
4. **ANSI 转义序列剥离**: 验证 `sanitize_terminal_output` 对复杂 ANSI 序列的处理
5. **`truncate_middle` 边界**: max_len = 0, 1, 2, 3, 4 的行为
6. **`is_agent_control_pane`**: 各种 pane title 的匹配/不匹配
7. **`detect_tool_call` 嵌套括号**: 如 `Bash(echo "hello (world)")` 的解析

---

## 四、文件清单

```
test-results/
├── 00_test_report.md          ← 本报告
├── 01_agent_control_plane_tests.txt  ← 单元测试输出
└── 02_status_bar_check.txt    ← 编译检查输出
```
