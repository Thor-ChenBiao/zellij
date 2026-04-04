# ACP 端到端测试用例 — 10 号段

**编写者**: Claude Opus  
**日期**: 2026-04-04  
**分支**: agent-control-plane-design  
**适用二进制**: `zellij` (确认包含 ACP 子命令)

---

## 前置条件

- macOS 终端（iTerm2 / Terminal.app / Alacritty 等）
- 已编译安装 `zellij`，`zellij --version` 输出 `0.44.0`
- `zellij claude --help` 能正常输出帮助信息
- 没有正在运行的同名 zellij session（测试前可 `zellij kill-all-sessions` 清理）
- 缓存目录可写（`zellij setup --check` 查看 `[CACHE DIR]`）
- 以下用 `$ACP_ROOT` 指代 `<CACHE DIR>/agent-control-plane/`

---

## TC-1001: `zellij claude` 无参数创建新房间

**前置**: 无同名 session 运行

**操作**:
1. 在终端直接执行 `zellij claude`

**预期**:
1. 终端先输出若干行启动信息（能看到），然后进入 Zellij UI
2. 启动信息中包含：
   - `Shared room: <id> (created)` — id 是一个随机生成的短字符串或数字
   - `Session name: <id>` — 与上面的 id 相同
   - `Endpoint: Claude Driver <endpoint-id> [claude / driver]`
   - `Share this room with another participant using: zellij reviewer <id>`
3. 进入 Zellij UI 后：
   - 顶部有 `tab-bar`
   - 中间 pane 里正在运行 `claude` 命令（Claude Code CLI）
   - 底部有原生 `status-bar`（显示 Zellij 快捷键提示）
   - 最底部有 `acp-bar`（一行）
4. `acp-bar` 显示内容应包含：
   - ` ACP ` 标签（高亮背景）
   - `room: <id>` 或 `<id>`
   - `self: claude/driver` 或 `claude/driver`
   - `driver(1) reviewer(0)` 或 `D1 R0`
5. tab 名称显示为 `ACP <id>`
6. `$ACP_ROOT/<id>/room.json` 文件存在，内容合法（包含 `schema_version`, `shared_id` 等）
7. `$ACP_ROOT/<id>/endpoints/` 下有一个 `claude-*.json` 文件，`role` 为 `"driver"`
8. `$ACP_ROOT/<id>/events.log` 至少有一行 `participant_registered`

**退出**: `Ctrl+Q` 或 `zellij kill-session <id>`  
**清理**: `rm -rf $ACP_ROOT/<id>`

---

## TC-1002: `zellij codex` 无参数创建新房间

**操作**: 与 TC-1001 相同，但执行 `zellij codex`

**预期差异**:
- 启动信息中 `Endpoint` 行显示 `Codex Driver`，provider 是 `codex`
- 中间 pane 执行的是 `codex` 命令（如果本机没装 codex，pane 里应显示 command not found）
- `acp-bar` 中 self 部分显示 `codex/driver`
- endpoint json 的 `provider` 是 `"codex"`

---

## TC-1003: `zellij gemini` 无参数创建新房间

**操作**: 与 TC-1001 相同，但执行 `zellij gemini`

**预期差异**:
- `Gemini Driver`，provider 是 `gemini`
- 中间 pane 执行 `gemini` 命令
- `acp-bar` 中显示 `gemini/driver`

---

## TC-1004: `zellij claude <id>` 带显式 id 创建新房间

**前置**: `$ACP_ROOT/my-test-room-1004/` 不存在

**操作**:
1. 执行 `zellij claude my-test-room-1004`

**预期**:
1. 启动信息显示 `Shared room: my-test-room-1004 (created)`
2. `Session name: my-test-room-1004`
3. tab 名为 `ACP my-test-room-1004`
4. `$ACP_ROOT/my-test-room-1004/room.json` 中 `shared_id` 为 `"my-test-room-1004"`
5. 其余与 TC-1001 一致

**清理**: `zellij kill-session my-test-room-1004 && rm -rf $ACP_ROOT/my-test-room-1004`

---

## TC-1005: `zellij claude <id>` 加入已有房间

**前置**: TC-1004 的 session 正在运行

**操作**:
1. 在**另一个终端窗口**执行 `zellij claude my-test-room-1004`

**预期**:
1. 启动信息显示 `Shared room: my-test-room-1004 (joined)`
2. 进入 Zellij UI 后，能看到与第一个窗口**相同的 session 内容**（Zellij attach 行为）
3. 房间里现在有一个新 pane 运行着 `claude`
4. `$ACP_ROOT/my-test-room-1004/endpoints/` 下现在有**两个** `claude-*.json` 文件
5. `acp-bar` 的 driver 计数更新为 `D2`（或 `driver(2)`）

---

## TC-1006: `zellij reviewer` 无参数创建新房间

**操作**:
1. 执行 `zellij reviewer`

**预期**:
1. 启动信息显示 `Shared room: <id> (created)`
2. `Endpoint: Reviewer <endpoint-id> [reviewer / reviewer]`
3. 接着显示 `Reviewer bootstrap:` 和一段 bootstrap prompt 文本
4. 再显示 `Review message template:` 和一段模板文本
5. **不显示** `Share this room with another participant` 这行（因为 reviewer 不是 driver）
6. 进入 UI 后，中间 pane 运行的是 reviewer 的 provider 命令（默认 `claude`）
7. `acp-bar` 显示 `reviewer/reviewer`，角色计数 `D0 R1`
8. endpoint json 中 `reviewer_bootstrap_prompt` 和 `review_message_template` 不为 null

---

## TC-1007: `zellij reviewer <id>` 加入已有 driver 房间

**前置**: TC-1004 的 session `my-test-room-1004` 正在运行（1 个 claude driver）

**操作**:
1. 在另一个终端执行 `zellij reviewer my-test-room-1004`

**预期**:
1. 启动信息显示 `Shared room: my-test-room-1004 (joined)`
2. 显示 reviewer bootstrap 和 message template
3. 进入 UI 后，能看到原有的 driver pane + 新打开的 reviewer pane
4. reviewer pane 中应该已经被**自动注入**了 bootstrap prompt 文本（以 `You are the reviewer` 开头），并且末尾有一个回车
5. `acp-bar` 的角色计数更新为 `D1 R1`（或 `driver(1) reviewer(1)`）
6. `events.log` 中有 `reviewer_bootstrap_injected` 事件
7. reviewer endpoint json 中 `bound_pane_id` 不为 null

**关键验证**: reviewer pane 中真的能看到注入的 bootstrap 文本，而不是空 pane

---

## TC-1008: `zellij view events <id>` 跟随事件流

**前置**: TC-1004 的 session `my-test-room-1004` 正在运行

**操作**:
1. 在另一个终端执行 `zellij view events my-test-room-1004`

**预期**:
1. 输出头部信息：
   - `Shared room: my-test-room-1004 | session: my-test-room-1004 | stream: events`
   - `Transport: local-room-cache | network: websocket-planned`
   - `Participants:` 后面列出所有已注册的 endpoint
   - `Following stream. Press Ctrl-C to stop.`
2. 接着输出 `events.log` 中**已有的所有历史事件**
3. 之后进入 follow 模式，终端**不退出**，光标停在最后
4. 如果此时在 driver pane 里做一些操作（比如打字），**新事件应该实时追加显示**

**关键验证**: 
- 不只是打印历史，还能实时跟踪（在 driver pane 打字后，view events 终端内能在 1 秒内看到对应的 `write_character` 事件）
- `Ctrl+C` 能正常退出

---

## TC-1009: `zellij view code <id>` 跟随代码流

**前置**: TC-1004 的 session `my-test-room-1004` 正在运行

**操作**:
1. 在另一个终端执行 `zellij view code my-test-room-1004`

**预期**:
1. 头部与 TC-1008 相同，但 stream 显示为 `code`
2. 输出 `code.log` 中的已有事件（通常包括 `pane_snapshot` 和 `room_ready`）
3. 进入 follow 模式
4. 当 driver pane 的内容发生变化时（如 Claude Code 输出新内容），能看到新的 `pane_snapshot` 事件

**关键验证**: 如果 driver pane 内容完全没变化，不应该产生重复的 `pane_snapshot`（去重逻辑）

---

## TC-1010: `zellij view events` 不带 id，在 Zellij session 内部运行

**前置**: 已经在 `my-test-room-1004` 的 Zellij session 内部

**操作**:
1. 在 session 中打开一个新 pane（`Ctrl+N` 或其他方式）
2. 在新 pane 中执行 `zellij view events`（不带任何 id）

**预期**:
1. 命令应该自动使用当前 session name 作为 shared_id（即 `my-test-room-1004`）
2. 正常输出事件流，与 TC-1008 相同的内容

**如果不在 Zellij session 内部运行且不带 id**:
1. 应该输出错误信息：`Please provide a shared room ID, or run this command inside a Zellij session.`
2. 退出码非零

---

## TC-1011: `zellij review request <id>` 生成 review request

**前置**: `my-test-room-1004` 正在运行，有 1 个 claude driver 和 1 个 reviewer

**操作**:
1. 在 driver pane 中做一些操作（确保 events.log 中有一些活动）
2. 在另一个终端执行 `zellij review request my-test-room-1004`

**预期**:
1. 输出：
   - `Request ID: rr-0001`（或递增的编号）
   - `Driver endpoint: claude-<hash>`
   - `Target pane: terminal_<N>`
2. 输出完整的 `[ACP_REVIEW_REQUEST_BEGIN]...[ACP_REVIEW_REQUEST_END]` block
3. block 中应包含从 room 流中汇总的：
   - `changed_files`（如果 driver 有编辑文件操作的话）
   - `last_commands`（如果 driver 有 Bash 命令的话）
   - `recent_output`
   - `recent_code_snapshot`（最近的 pane viewport 内容）
4. 输出持久化路径：
   - `Saved request JSON: $ACP_ROOT/my-test-room-1004/review/requests/latest.json`
   - `Saved request text: $ACP_ROOT/my-test-room-1004/review/requests/latest.txt`
5. 如果 reviewer pane 还活着：`Injected review request into reviewer <endpoint-id> (<pane-id>)`
6. 如果 reviewer pane 不在了：`No live reviewer panes were found. Request kept on disk.`
7. 去 reviewer 的 pane 里看，应该能看到 request 文本**已经被粘贴进去了**

**关键验证**: 
- request 中的 `target_pane_id` 应该指向 driver 的 pane，而不是 reviewer 的 pane 或 plugin pane
- `recent_code_snapshot` 应该是 driver pane 的 viewport 内容，不是 reviewer pane 的
- review request 被注入到 reviewer pane 后，reviewer agent 能看到并处理它

---

## TC-1012: `zellij review feedback <id>` 从 stdin 记录 feedback

**前置**: TC-1011 已经执行过（有 `rr-0001` request）

**操作**:
1. 在另一个终端执行：
```bash
cat <<'EOF' | zellij review feedback my-test-room-1004
[ACP_REVIEW_RESPONSE_V1]
room_id: my-test-room-1004
request_id: rr-0001
source_endpoint: reviewer-test
target_role: driver
severity: medium
confidence: high
should_send: true
summary: 需要补充测试用例。

findings:
- 没有发现回归测试。

actions:
- 补充一个测试用例。

notes:
- 改动很小。
[/ACP_REVIEW_RESPONSE_V1]
EOF
```

**预期**:
1. 输出：
   - `Request ID: rr-0001`
   - `Source endpoint: reviewer-test`
   - `Will send to driver: true`
   - `Target pane: terminal_<N>`（与 TC-1011 的 target pane 一致）
2. 输出完整的 `[ACP_REVIEW_RESPONSE_V1]...[/ACP_REVIEW_RESPONSE_V1]` 回显
3. 持久化路径：
   - `Saved feedback JSON: ...feedback/latest.json`
   - `Saved feedback text: ...feedback/latest.txt`
   - `Saved driver envelope: ...driver-inbox/latest.txt`
4. 如果 driver session 活跃且 pane 可定位：`Injected driver envelope into: terminal_<N>`
5. 去 driver pane 里看，应该能看到注入的 `[ACP_MESSAGE_BEGIN]...[ACP_MESSAGE_END]` envelope 文本
6. envelope 文本末尾应该还有 `[Review from reviewer-test]` 和 findings/actions 列表

**关键验证**:
- driver pane 中确实出现了 review envelope 文本
- envelope 是以 paste 方式注入的，后面跟了一个 Enter
- 如果 driver 是 Claude Code，Claude Code 应该能把这段注入的文本当作新的用户输入来处理

---

## TC-1013: `zellij review feedback` 从文件读取（`--file`）

**前置**: TC-1011 已执行

**操作**:
1. 先将 feedback 内容写入文件 `/tmp/test-feedback.txt`
2. 执行 `zellij review feedback my-test-room-1004 --file /tmp/test-feedback.txt`

**预期**: 与 TC-1012 相同（只是输入源不同）

---

## TC-1014: `zellij review feedback` should_send: false

**前置**: TC-1011 已执行

**操作**:
```bash
cat <<'EOF' | zellij review feedback my-test-room-1004
[ACP_REVIEW_RESPONSE_V1]
room_id: my-test-room-1004
request_id: rr-0001
source_endpoint: reviewer-test
target_role: driver
severity: low
confidence: low
should_send: false
summary: 没什么问题。

findings:
- 一切正常。

actions:
- 无

notes:
- skip
[/ACP_REVIEW_RESPONSE_V1]
EOF
```

**预期**:
1. 输出 `Will send to driver: false`
2. **不输出** `Saved driver envelope` 行
3. **不输出** `Injected driver envelope` 行
4. driver pane 中**不应该出现**任何新的注入文本
5. feedback JSON 和 text 仍然正常保存

---

## TC-1015: 普通 session 中输入 `claude` 自动升级为 ACP room

**前置**: 没有正在运行的 ACP room

**操作**:
1. 执行 `zellij` 启动一个普通 session（不带任何 ACP 参数）
2. 观察初始状态：tab 名应该是 `Tab #1`
3. 在 pane 中输入 `claude` 然后按回车

**预期**:
1. `claude` 命令正常启动
2. **tab 名从 `Tab #1` 变成 `ACP <auto-room-id>`**（auto-room-id 类似 `auto-room-200005`）
3. `$ACP_ROOT/<auto-room-id>/` 目录被创建
4. `events.log` 中包含 `session_upgraded_to_acp_room` 和 `participant_registered` 和 `driver_auto_detected` 事件
5. 当前 pane 被绑定为 driver（endpoint json 中 `bound_pane_id` 不为 null）

**关键验证**:
- tab 名真的从默认名变过来了
- 如果 tab 名已经被用户手动改过（不是 `Tab #N`），则不应该被覆盖

---

## TC-1016: 普通 session 中输入 `codex` 自动升级

与 TC-1015 相同，但输入 `codex`。预期 provider 为 `codex`。

---

## TC-1017: 普通 session 中输入 `gemini` 自动升级

与 TC-1015 相同，但输入 `gemini`。预期 provider 为 `gemini`。

---

## TC-1018: `acp-bar` 在 reviewer 加入后动态更新角色计数

**前置**: `zellij claude my-test-room-1018` 正在运行

**操作**:
1. 观察 `acp-bar`，确认显示 `D1 R0` 或 `driver(1) reviewer(0)`
2. 在另一个终端执行 `zellij reviewer my-test-room-1018`
3. 等待 reviewer 加入完成
4. 回到第一个终端窗口，观察 `acp-bar`

**预期**:
1. `acp-bar` 的角色计数从 `D1 R0` 更新为 `D1 R1`
2. 更新应该在 reviewer 加入后几秒内发生

**关键验证**: 角色计数真的动态刷新了，而不是停在旧值

---

## TC-1019: reviewer bootstrap 文本注入到 reviewer pane 的完整性

**前置**: `zellij claude my-test-room-1019` 正在运行

**操作**:
1. 在另一个终端执行 `zellij reviewer my-test-room-1019`
2. 进入 session 后，找到 reviewer 的 pane

**预期**:
1. reviewer pane 中可以看到被注入的 bootstrap 文本，应包含：
   - `You are the reviewer for shared room`
   - `Identity:` 段落，包含 `room_id`, `reviewer_endpoint`, `role: reviewer`
   - `[ACP_REVIEW_RESPONSE_V1]` 模板格式说明
   - `Reply once with READY as the reviewer, then wait for review requests.`
2. 文本的最后有一个回车（即 bootstrap 已经作为输入提交了）
3. 如果 reviewer pane 运行的是 Claude Code，Claude Code 应该已经开始处理这段 bootstrap 文本

**关键验证**: 注入的文本完整、格式正确、没有被截断

---

## TC-1020: 布局验证 — tab-bar / provider pane / status-bar / acp-bar 四层结构

**操作**:
1. 执行 `zellij claude`

**预期**:
1. 从上到下应有四层：
   - **最顶部**: tab-bar（Zellij 原生 tab 切换栏）
   - **中间最大区域**: provider pane（运行 `claude`）
   - **倒数第二行**: status-bar（Zellij 原生快捷键提示，如 `Ctrl+N New Pane` 等）
   - **最底部一行**: acp-bar（`ACP` 标签 + 房间信息）
2. status-bar 应该是**完整的 Zellij 原生快捷键栏**，不应该被 ACP 信息覆盖或替换
3. acp-bar 是独立的一行，不是嵌入在 status-bar 内部的

**关键验证**: 
- status-bar 和 acp-bar 是**两个独立的行**
- 快捷键（如 `Ctrl+Q` 退出）仍然正常工作

---

## TC-1021: tab 名称显示 `ACP <id>`

**操作**:
1. `zellij claude test-tab-name`

**预期**:
1. tab-bar 中的 tab 名称显示为 `ACP test-tab-name`
2. 不是显示为 `test-tab-name` 也不是 `Tab #1`

---

## TC-1022: 多个 driver 加入同一房间后 endpoint 不互相覆盖

**前置**: `zellij claude multi-driver-test` 正在运行

**操作**:
1. 在另一个终端执行 `zellij codex multi-driver-test`

**预期**:
1. `$ACP_ROOT/multi-driver-test/endpoints/` 下有至少两个文件
2. 一个 provider 为 `claude`，另一个为 `codex`
3. 它们的 `endpoint_id` 不同
4. 它们的 `bound_pane_id` 不同（各绑到自己的 pane）

---

## TC-1023: `zellij review request` 区分 driver 和 reviewer 事件

**前置**: 
- `zellij claude mixed-room` 正在运行（driver 在工作）
- `zellij reviewer mixed-room` 已加入

**操作**:
1. 在 driver pane 中做一些操作（触发 tool_call 等事件）
2. 执行 `zellij review request mixed-room`

**预期**:
1. `changed_files` 只包含 driver 触碰的文件，不包含 reviewer 的活动
2. `last_commands` 只包含 driver 执行的命令
3. `recent_code_snapshot` 是 driver pane 的 viewport，不是 reviewer pane 的
4. `target_pane_id` 指向 driver 的 pane

**关键验证**: review request 中不应该混入 reviewer 自己的活动数据

---

## TC-1024: feedback 错误处理 — 缺少 ACP marker

**操作**:
```bash
echo "This is garbage" | zellij review feedback my-test-room
```

**预期**:
1. stderr 输出 `Failed to record review feedback ... Missing ACP review response start marker`
2. 退出码非零

---

## TC-1025: feedback 错误处理 — 缺少必需字段

**操作**:
```bash
cat <<'EOF' | zellij review feedback my-test-room
[ACP_REVIEW_RESPONSE_V1]
room_id: my-test-room
severity: low
[/ACP_REVIEW_RESPONSE_V1]
EOF
```

**预期**:
1. stderr 输出 `Feedback is missing request_id`
2. 退出码非零

---

## TC-1026: feedback 错误处理 — room_id 不匹配

**操作**:
```bash
cat <<'EOF' | zellij review feedback my-test-room
[ACP_REVIEW_RESPONSE_V1]
room_id: wrong-room
request_id: rr-0001
source_endpoint: test
summary: test
[/ACP_REVIEW_RESPONSE_V1]
EOF
```

**预期**:
1. stderr 输出包含 `does not match target room`
2. 退出码非零

---

## TC-1027: feedback 错误处理 — 空输入

**操作**:
```bash
echo "" | zellij review feedback my-test-room
```

**预期**:
1. stderr 输出 `No feedback content provided`
2. 退出码非零

---

## TC-1028: `zellij review request` 对不存在的房间

**操作**:
```bash
zellij review request nonexistent-room-xyz
```

**预期**:
1. stderr 输出错误（如 `Failed to prepare review request ... No such file or directory`）
2. 退出码非零
3. 不应该自动创建房间

---

## TC-1029: `zellij view events` 对不存在的房间

**操作**:
```bash
zellij view events brand-new-room-xyz
```

**预期**:
1. 验证行为：是报错退出，还是自动创建空房间并开始 follow？
2. 如果自动创建：`$ACP_ROOT/brand-new-room-xyz/` 目录会被创建，这可能不是用户预期
3. **记录实际行为**（这是一个潜在的设计决策问题）

---

## TC-1030: 事件流实时性验证

**前置**: `zellij claude realtime-test` 正在运行

**操作**:
1. 在另一个终端运行 `zellij view events realtime-test`
2. 回到 driver pane，输入一些字符（如 `hello world`）
3. 观察 view events 终端

**预期**:
1. 在 view events 终端中，应该在 1 秒内看到对应的 `write_character` 事件
2. 事件格式为 `<timestamp> | room | write_character | Input <chars>`

**关键验证**: 延迟在可接受范围内（< 1 秒）

---

## TC-1031: code 流去重验证

**前置**: `zellij claude dedup-test` 正在运行

**操作**:
1. 在另一个终端运行 `zellij view code dedup-test`
2. 等待 10 秒，期间 driver pane 不做任何操作

**预期**:
1. 如果 pane 内容没有变化，不应该持续产生新的 `pane_snapshot` 事件
2. code.log 不应该被相同内容的 snapshot 无限刷屏

---

## TC-1032: `zellij review request` 连续多次执行

**前置**: `zellij claude repeat-request` 正在运行

**操作**:
1. 执行 `zellij review request repeat-request`
2. 再执行一次 `zellij review request repeat-request`

**预期**:
1. 第一次输出 `Request ID: rr-0001`
2. 第二次输出 `Request ID: rr-0002`（递增）
3. `$ACP_ROOT/repeat-request/review/requests/history.jsonl` 中有两行
4. `latest.json` 是最新的 request

---

## TC-1033: 在 Zellij session 内部执行 `zellij claude <same-session-id>`

**前置**: 已经在 `my-room` 的 Zellij session 内部

**操作**:
1. 在 session 内打开新 pane
2. 在新 pane 中执行 `zellij claude my-room`

**预期**:
1. 应该**不会**尝试创建新 session（因为已经在目标 session 内部）
2. 应该在当前 session 中新开一个运行 `claude` 的 pane
3. 启动时的 launcher pane（执行命令的那个）可能会被关闭
4. 不应该出现嵌套的 Zellij session

**关键验证**: 不会嵌套启动 Zellij

---

## TC-1034: 在 Zellij session 内部执行 `zellij reviewer <same-session-id>`

与 TC-1033 类似，但执行 `zellij reviewer my-room`。

**额外验证**: 
- 新的 reviewer pane 创建成功
- reviewer bootstrap 被注入到新 pane
- launcher pane 被关闭（如果存在）

---

## TC-1035: driver endpoint 的 pane binding 持久化

**前置**: `zellij claude binding-test` 正在运行

**操作**:
1. 执行 `zellij review request binding-test`
2. 检查 `$ACP_ROOT/binding-test/endpoints/claude-*.json`

**预期**:
1. `bound_pane_id` 字段不为 null（例如 `"terminal_0"`）
2. `bound_pane_id_updated_at` 不为 null
3. 这个 pane_id 对应的确实是 driver 的 pane（不是 plugin 或 reviewer 的 pane）

---

## TC-1036: review feedback 注入后 driver pane 的实际效果

**前置**: 
- `zellij claude inject-test` 正在运行，driver pane 内 Claude Code 已启动
- 已执行 `zellij review request inject-test`

**操作**:
1. 执行 feedback（should_send: true）
2. 切换到 driver pane 观察

**预期**:
1. driver pane（Claude Code）的输入区域中出现了 `[ACP_MESSAGE_BEGIN]...[ACP_MESSAGE_END]` 的文本
2. 文本后面有 `[Review from ...]` 和具体 findings
3. 因为注入后还发了 Enter，Claude Code 可能已经开始处理这段文本作为新的用户消息

**关键验证**:
- 注入的文本是完整的，没有被截断
- 如果 Claude Code 正在思考/输出中，注入可能会失败或产生意外效果（记录实际行为）

---

## TC-1037: `Ctrl+Q` 退出 ACP session 后房间数据保留

**操作**:
1. `zellij claude persist-test`
2. `Ctrl+Q` 退出
3. 检查 `$ACP_ROOT/persist-test/`

**预期**:
1. 退出后，房间目录仍然存在
2. `room.json`、`events.log`、`code.log`、endpoint json 全部保留
3. 数据不会因为 session 退出而被删除

---

## TC-1038: `zellij review feedback --source-endpoint` 覆盖

**前置**: TC-1011 已执行

**操作**:
```bash
cat <<'EOF' | zellij review feedback my-test-room --source-endpoint custom-reviewer-42
[ACP_REVIEW_RESPONSE_V1]
room_id: my-test-room
request_id: rr-0001
target_role: driver
severity: low
confidence: low
should_send: false
summary: 测试。

findings:
- 测试。

actions:
- 无。

notes:
- 无。
[/ACP_REVIEW_RESPONSE_V1]
EOF
```

**预期**:
1. 输出 `Source endpoint: custom-reviewer-42`（使用了 `--source-endpoint` 指定的值，而不是 block 中的值）
2. 即使 block 中没有 `source_endpoint` 字段也能成功

---

## TC-1039: 中文和特殊字符在 room id 中的行为

**操作**:
```bash
zellij claude "test-中文-room"
```

**预期**:
1. 如果 Zellij 的 session name 验证允许中文，应该正常创建
2. 如果不允许，应该给出明确的错误信息
3. **记录实际行为**

---

## TC-1040: 非常长的 room id

**操作**:
```bash
zellij claude "$(python3 -c 'print("a"*200)')"
```

**预期**:
1. 要么正常创建（如果允许长名称）
2. 要么给出明确错误
3. 不应该 panic 或产生未定义行为

---

## TC-1041: 并发创建同 id 的房间

**操作**:
1. 在两个终端**同时**执行 `zellij claude same-room-race`

**预期**:
1. 其中一个显示 `(created)`，另一个显示 `(joined)` 或也 `(created)` 然后 attach
2. 不应该出现 panic、数据损坏或文件锁冲突
3. 最终 `$ACP_ROOT/same-room-race/room.json` 文件格式合法

---

## TC-1042: review request 在没有任何 driver 活动时的输出

**前置**: `zellij claude empty-room` 刚启动，driver 什么都没做

**操作**:
1. 立刻执行 `zellij review request empty-room`

**预期**:
1. 正常输出 request（不应该报错）
2. `changed_files: <none>`
3. `last_commands: <none>`
4. `recent_output: <none>` 或非常少的内容
5. `recent_code_snapshot` 可能有初始的 pane viewport 内容（如 shell prompt 或 Claude Code 欢迎界面）

---

## TC-1043: view events 在房间为空时的行为

**前置**: 创建一个房间但不启动 session（直接用 API 创建 room.json）

**操作**:
```bash
mkdir -p $ACP_ROOT/manual-room/endpoints
echo '{"schema_version":1,"shared_id":"manual-room","session_name":"manual-room","created_at":"2026-04-04T00:00:00Z","updated_at":"2026-04-04T00:00:00Z","zellij_version":"0.44.0","bridge_transport":"local-room-cache","network_transport":"websocket-planned"}' > $ACP_ROOT/manual-room/room.json
touch $ACP_ROOT/manual-room/events.log $ACP_ROOT/manual-room/events.jsonl $ACP_ROOT/manual-room/code.log $ACP_ROOT/manual-room/code.jsonl
zellij view events manual-room
```

**预期**:
1. 正常启动，显示头部信息
2. Participants 列表为空
3. 进入 follow 模式，等待新事件

---

## TC-1044: 在 `zellij claude` session 中使用 `Ctrl+N` 开新 pane

**前置**: `zellij claude test-new-pane` 正在运行

**操作**:
1. 按 `Ctrl+N`（Zellij 默认打开新 pane 的快捷键）

**预期**:
1. 新 pane 正常打开（Zellij 原生功能不受 ACP 影响）
2. 新 pane 是一个普通 shell
3. status-bar 上的快捷键仍然正常工作

**关键验证**: ACP 不会破坏 Zellij 的基本功能

---

## TC-1045: 杀掉 session 后再用同 id 重新创建

**操作**:
1. `zellij claude reuse-id-test`
2. `Ctrl+Q` 退出
3. `zellij kill-session reuse-id-test`（如果还在的话）
4. 再次 `zellij claude reuse-id-test`

**预期**:
1. 第二次启动应显示 `(joined)` 或 `(created)`（取决于 room.json 是否还存在）
2. session 正常启动，不会因为旧数据导致异常
3. 旧的 endpoint 数据不会干扰新 session

---

## 测试结果汇总模板

| 用例 | 标题 | 结果 | 备注 |
|------|------|------|------|
| TC-1001 | claude 无参数创建房间 | | |
| TC-1002 | codex 无参数创建房间 | | |
| TC-1003 | gemini 无参数创建房间 | | |
| TC-1004 | claude 带 id 创建房间 | | |
| TC-1005 | claude 带 id 加入已有房间 | | |
| TC-1006 | reviewer 无参数创建房间 | | |
| TC-1007 | reviewer 加入 driver 房间 | | |
| TC-1008 | view events 跟随事件流 | | |
| TC-1009 | view code 跟随代码流 | | |
| TC-1010 | view events 不带 id（session 内部） | | |
| TC-1011 | review request 生成 | | |
| TC-1012 | review feedback stdin 输入 | | |
| TC-1013 | review feedback --file 输入 | | |
| TC-1014 | review feedback should_send false | | |
| TC-1015 | 普通 session 输入 claude 自动升级 | | |
| TC-1016 | 普通 session 输入 codex 自动升级 | | |
| TC-1017 | 普通 session 输入 gemini 自动升级 | | |
| TC-1018 | acp-bar 动态更新角色计数 | | |
| TC-1019 | reviewer bootstrap 注入完整性 | | |
| TC-1020 | 四层布局结构验证 | | |
| TC-1021 | tab 名称 ACP <id> | | |
| TC-1022 | 多 driver 不互相覆盖 | | |
| TC-1023 | review request 区分 driver/reviewer | | |
| TC-1024 | feedback 错误 — 缺 marker | | |
| TC-1025 | feedback 错误 — 缺必需字段 | | |
| TC-1026 | feedback 错误 — room_id 不匹配 | | |
| TC-1027 | feedback 错误 — 空输入 | | |
| TC-1028 | review request 不存在的房间 | | |
| TC-1029 | view events 不存在的房间 | | |
| TC-1030 | 事件流实时性 | | |
| TC-1031 | code 流去重 | | |
| TC-1032 | review request 连续多次 | | |
| TC-1033 | session 内部执行 claude (同 id) | | |
| TC-1034 | session 内部执行 reviewer (同 id) | | |
| TC-1035 | driver pane binding 持久化 | | |
| TC-1036 | feedback 注入后 driver pane 效果 | | |
| TC-1037 | 退出后房间数据保留 | | |
| TC-1038 | --source-endpoint 覆盖 | | |
| TC-1039 | 中文 room id | | |
| TC-1040 | 超长 room id | | |
| TC-1041 | 并发创建同 id 房间 | | |
| TC-1042 | 空房间 review request | | |
| TC-1043 | 空房间 view events | | |
| TC-1044 | ACP session 中 Ctrl+N 开新 pane | | |
| TC-1045 | 杀 session 后重用 id | | |
