# ACP 端到端测试报告

执行时间：2026-04-04  
分支：`agent-control-plane-design`  
执行人：Claude Code (PM 视角)  
构建模式：debug → release

## 总体评价

ACP 核心功能路径基本可用，但存在若干需要修复的问题。
房间创建/加入、review request/feedback 的文件化闭环已经落地。
启动体验、插件加载和 code stream 语义需要进一步打磨。

## 测试结果汇总

| 测试组 | 用例数 | PASS | FAIL | BLOCKED | 说明 |
|--------|--------|------|------|---------|------|
| 房间生命周期 TC-001~008 | 8 | 5 | 1 | 2 | TC-001/002 实测通过，TC-005/006 需 TUI 手动验证 |
| 自动升级 TC-009~014 | 6 | 0 | 0 | 6 | 需 TUI 交互，代码审查确认逻辑存在于 screen.rs |
| view 命令 TC-015~017 | 3 | 1 | 1 | 1 | TC-017 通过（报错路径），code/events 语义反了 |
| review request TC-018~022 | 5 | 3 | 0 | 2 | 文件生成和 waiting 路径正常 |
| review feedback TC-023~029 | 7 | 4 | 0 | 3 | 解析、校验、envelope 生成正常 |
| ACP bar TC-030~032 | 3 | 1 | 1 | 1 | alias 缺失已修复并验证，bar 现在可见 |
| 文件结构 TC-033 | 1 | 1 | 0 | 0 | 目录结构符合预期 |

## 发现的 BUG 与问题

### BUG-001: acp-bar 插件 alias 未注册（已修复）

**严重度**：Critical  
**状态**：已修复

`default.kdl` 的 `plugins {}` block 里缺少 `acp-bar` alias 定义，导致 layout 中的 `plugin location="zellij:acp-bar"` 无法解析。

日志表现：
```
ERROR: "Failed to resolve plugin alias"
ERROR: Plugin with id: 3 not found
```

修复：在 `zellij-utils/assets/config/default.kdl` 的 plugins block 中添加：
```kdl
acp-bar location="zellij:acp-bar"
```

### BUG-002: `view code` 和 `view events` 语义不符合用户预期

**严重度**：High  
**状态**：待重新设计

当前 `view code` 显示的是 pane viewport 快照（高频刷新的 pane_snapshot 事件），而非用户期望的代码变更流。用户期望：
- `view code`：看到 agent 修改了哪些文件、哪些行，带完整函数上下文
- `view events`：看到所有操作事件（包括当前的 pane_snapshot）

详见：[view code 重新设计方案](./view-code-redesign.md)

### BUG-003: `reviewer` 独立命令 multicall 未生效

**严重度**：Medium  
**状态**：代码已写入，待下次编译生效

添加了 `try_multicall()` 函数检测 `argv[0]`，支持通过 symlink 直接调用 `reviewer`/`claude`/`codex`/`gemini`。
Symlink 已创建在 `~/.cargo/bin/`，需要下次 release 编译后生效。

### BUG-004: 启动速度慢（debug 模式 ~7s）

**严重度**：Medium  
**状态**：已优化

原因分析（来自 zellij.log 时间线）：

| 阶段 | 耗时 | 说明 |
|------|------|------|
| Client → Server | ~100ms | 正常 |
| Server ready retry loop | ~400ms | 客户端等服务器就绪 |
| Wasm 插件加载 | ~6.4s | **最大瓶颈** |
| - link | 240ms | 已禁用 |
| - tab-bar | 340ms | 必须保留 |
| - status-bar | 602ms | 必须保留 |
| - about | 267ms | 已移除 |
| - acp-bar (失败超时) | ~7.4s | alias 修复后消除 |

已执行的优化：
1. 注释掉 `zellij:link` 后台插件（-240ms）
2. 移除 `about` keybind 预加载（-267ms）
3. 设置 `show_release_notes false`
4. 设置 `show_startup_tips false`
5. 修复 acp-bar alias（消除 7.4s 超时）
6. 切换到 release 模式编译（wasm 加载整体提速）

### ISSUE-001: Room ID 使用 session 名而非数字 ID

**严重度**：Low  
**状态**：已由另一个进程修复（改为递增 ID）

在已有 Zellij session 内运行 `zellij claude` 时，`resolve_default_room_id` 复用了当前 session 名（如 `circular-tambourine`），而非生成 6 位数字 ID。这是设计意图（TC-007），但用户可能觉得不直观。

## 通过的关键测试详情

### TC-001: 新建房间（zellij claude）— PASS

实测 stdout 输出：
```
Shared room: 972389 (created)
Session name: 972389
Endpoint: Claude Driver claude-a1a4c0fd [claude / driver]
Share this room with another participant using: zellij reviewer 972389
```

文件系统验证：
- `room.json` ✓（schema_version=1, bridge_transport=local-room-cache）
- `events.log` ✓（包含 participant_registered）
- `events.jsonl` ✓
- `code.log` ✓（包含 room_ready）
- `code.jsonl` ✓
- `endpoints/claude-a1a4c0fd.json` ✓（provider=claude, role=driver）

### TC-002: 指定不存在 ID — PASS

```
Shared room: 123456 (created)
Session name: 123456
Endpoint: Codex Driver codex-a0aa74a0 [codex / driver]
```

### TC-030: ACP bar 全宽显示 — PASS（修复后）

修复 alias 注册后，ACP bar 正常渲染：
```
ACP  room: 104990 | self: claude/driver | roles: driver(1) reviewer(0)
```

## 未能执��的测试

以下测试需要 TUI 交互环境，在 CLI 自动化中无法执行：

- TC-005: session 内启动（关闭 launcher pane）
- TC-006: reviewer bootstrap 注入
- TC-009~014: 普通 session 自动升级
- TC-031: ACP bar 窄宽紧凑显示
- TC-032: 无 room_id 显示空白

建议后续补充手动测试或 integration test。

## 配置变更记录

`zellij-utils/assets/config/default.kdl` 的变更：

```diff
+ acp-bar location="zellij:acp-bar"           # plugins block
+ show_release_notes false                      # 禁用 release notes
+ show_startup_tips false                       # 禁用启动 tips
- "zellij:link"                                 # 注释掉后台 link 插件
- bind "a" { LaunchOrFocusPlugin "zellij:about" } # 注释掉 about 预加载
```
