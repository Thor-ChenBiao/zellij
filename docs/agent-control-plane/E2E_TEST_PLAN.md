# Agent Control Plane 端到端测试用例文档（路径覆盖版）

更新时间：2026-04-04  
适用分支：`agent-control-plane-design`

## 1. 目标

本测试文档用于手工端到端验证 ACP（Agent Control Plane）在当前分支的行为。

覆盖目标：
1. 覆盖所有用户可见命令路径（`claude/codex/gemini/reviewer/view/review`）。
2. 覆盖所有关键状态分支（新建房间、加入房间、普通 session 自动升级、review request/feedback 注入与等待）。
3. 覆盖关键失败路径（输入不合法、目标 pane 不可达、session 不活跃、注入失败等待）。

说明：这里的“100% 路径覆盖”指 ACP 相关功能路径覆盖，不包含 Zellij 全仓库其它非 ACP 功能。

## 2. 测试范围

包含模块：
1. `src/main.rs` ACP 命令分流。
2. `src/commands.rs` 房间创建/加入、pane 拉起、review request/feedback 流程。
3. `src/agent_control_plane.rs` 房间元数据、事件流、review 请求/反馈持久化。
4. `zellij-server/src/agent_control_plane.rs` PTY/输入启发式捕获。
5. `zellij-server/src/screen.rs` 普通 session 自动升级 ACP、tab 自动命名、acp-room-state 推送。
6. `default-plugins/acp-bar/src/main.rs` 房间状态显示。

不包含：
1. 远程 Web transport（当前设计预留，未落地）。
2. reviewer 自动触发闭环（当前仍是手动 `review request` / `review feedback` 驱动）。

## 3. 测试前准备

## 3.1 环境前提

1. 已有可执行 `zellij` 二进制，且来自本分支构建产物。
2. 终端支持交互式 TUI（建议本机终端，不建议高延迟 SSH）。
3. 本文档不要求运行 `cargo test`，只做黑盒 E2E 手测。

## 3.2 路径变量

测试过程中会频繁检查 ACP 缓存目录，统一记为：

`<ACP_CACHE_ROOT>=<ZELLIJ_CACHE_DIR>/agent-control-plane`

房间目录模板：

`<ROOM_DIR>=<ACP_CACHE_ROOT>/<room_id>`

## 3.3 基础观测命令

执行测试时建议准备第二个 shell 用于观测：

```bash
# 观察房间目录
ls -la <ROOM_DIR>

# 观察事件流文件
tail -f <ROOM_DIR>/events.log

# 观察代码线索流
tail -f <ROOM_DIR>/code.log

# 观察 endpoint 元数据
ls -la <ROOM_DIR>/endpoints
cat <ROOM_DIR>/endpoints/<endpoint_id>.json
```

## 4. 覆盖矩阵（路径 -> 用例）

| 路径ID | 代码路径 | 覆盖用例 |
|---|---|---|
| P01 | `start_shared_room`: 无 ID，创建新房间 | TC-001 |
| P02 | `start_shared_room`: 指定 ID，不存在时创建 | TC-002 |
| P03 | `start_shared_room`: 指定 ID，存在时加入 | TC-003 |
| P04 | `start_shared_room`: 房间活跃时拉起新 pane | TC-004 |
| P05 | `start_shared_room`: 在目标 session 内启动，focus 新 pane 并关闭 launcher pane | TC-005 |
| P06 | `start_shared_room`: reviewer 启动并注入 bootstrap | TC-006 |
| P07 | `resolve_default_room_id`: 在 session 内无 ID 时使用 session 名 | TC-007 |
| P08 | `resolve_default_room_id`: 无上下文时生成 6 位 ID | TC-008 |
| P09 | `screen` 输入捕获：普通 session 输入 `claude/codex/gemini` 自动升级 | TC-009/010/011 |
| P10 | `screen` 输入捕获：`zellij reviewer/view/review` 触发 EnsureRoom | TC-012 |
| P11 | `screen` tab 自动命名：默认 `Tab #N` -> `ACP <id>` | TC-013 |
| P12 | `screen` tab 自动命名：非默认 tab 名保持不变 | TC-014 |
| P13 | `view events/code`: 显式 room_id | TC-015/016 |
| P14 | `view events/code`: 无 ID 且不在 session 内报错 | TC-017 |
| P15 | `prepare_review_request`: 生成请求并落盘 | TC-018 |
| P16 | `prepare_review_request`: 活跃 reviewer 可注入 | TC-019 |
| P17 | `prepare_review_request`: 无 live reviewer -> waiting 事件 | TC-020 |
| P18 | `prepare_review_request`: session 不活跃只落盘 | TC-021 |
| P19 | `prepare_review_request`: backfill driver pane binding | TC-022 |
| P20 | `record_review_feedback`: should_send=true 且 pane 可解 -> 注入 driver | TC-023 |
| P21 | `record_review_feedback`: should_send=true 且 session 不活跃 -> waiting | TC-024 |
| P22 | `record_review_feedback`: should_send=true 且 pane 不可解 -> waiting | TC-025 |
| P23 | `record_review_feedback`: should_send=false，不产 envelope | TC-026 |
| P24 | `record_review_feedback`: 缺失 ACP block marker 报错 | TC-027 |
| P25 | `record_review_feedback`: room_id 不匹配报错 | TC-028 |
| P26 | `record_review_feedback`: source_endpoint 缺失并用 override 成功 | TC-029 |
| P27 | `acp-bar` 全宽显示 | TC-030 |
| P28 | `acp-bar` 窄宽紧凑显示 | TC-031 |
| P29 | `acp-bar` 无 room_id 时空白 | TC-032 |
| P30 | 房间与 review 文件结构完整性 | TC-033 |

## 5. 端到端测试用例

## 5.1 房间生命周期与命令入口

### TC-001 新建房间（`zellij claude`）

目标：覆盖新建房间路径。  
步骤：
1. 在非 Zellij shell 执行：`zellij claude`。
2. 记录 stdout 的 room_id。

预期输出：
1. stdout 包含 `Shared room: <room_id> (created)`。
2. stdout 包含 `Session name: <room_id>`。
3. stdout 包含 `Endpoint: Claude Driver ... [claude / driver]`。
4. stdout 包含 `Share this room with another participant using: zellij reviewer <room_id>`。

预期状态：
1. 打开新 session，tab 名为 `ACP <room_id>`。
2. `<ROOM_DIR>/room.json`、`events.log/jsonl`、`code.log/jsonl` 已创建。
3. `<ROOM_DIR>/endpoints` 下有一个 driver endpoint json。

---

### TC-002 指定不存在 ID（创建）

步骤：
1. 在非 Zellij shell 执行：`zellij codex 123456`（确保该 ID 不存在）。

预期：
1. stdout 显示 `(created)`。
2. provider 为 `codex`，role 为 `driver`。
3. 新 session 名是 `123456`。

---

### TC-003 指定存在 ID（加入）

步骤：
1. 先执行 TC-001 创建房间。
2. 在新 shell 执行：`zellij gemini <room_id>`。

预期：
1. stdout 显示 `(joined)`。
2. endpoint 新增一条 `gemini driver`。
3. `events.log` 有新增 `participant_registered`。

---

### TC-004 房间已活跃时拉起新 pane

步骤：
1. 保持 `<room_id>` session 运行中。
2. 在外部 shell 执行：`zellij reviewer <room_id>`。

预期：
1. 不创建新 session，而是在现有 session 内新增 pane。
2. `<ROOM_DIR>/endpoints` 新增 reviewer endpoint。
3. `events.log` 出现 `participant_registered` 与 reviewer 相关事件。

---

### TC-005 在目标 session 内启动（关闭 launcher pane）

步骤：
1. 进入 ACP session `<room_id>`。
2. 在当前 pane 内执行：`zellij reviewer <room_id>`。

预期：
1. 新 reviewer pane 拉起并获得焦点。
2. 当前启动命令的 launcher pane 被关闭（不会残留一个无用 pane）。

---

### TC-006 reviewer bootstrap 注入

步骤：
1. 在活跃 room 执行：`zellij reviewer <room_id>`。
2. 切到 reviewer pane。

预期：
1. pane 内自动出现 bootstrap prompt + `ACP_REVIEW_RESPONSE_V1` 协议模板。
2. `events.log` 包含 `reviewer_bootstrap_injected`。

---

### TC-007 session 内无 ID 默认复用 session 名

步骤：
1. 启动普通 session，session 名设为 `my-room`。
2. 在此 session 内执行：`zellij claude`（不带 ID）。

预期：
1. room_id 解析为 `my-room`。
2. `<ACP_CACHE_ROOT>/my-room` 被创建/复用。

---

### TC-008 无上下文时生成 6 位数字 room_id

步骤：
1. 在非 Zellij shell 连续执行数次 `zellij claude`。

预期：
1. room_id 为 6 位数字。
2. 基本不冲突（同秒多次也应通过 offset 找到空闲 ID）。

## 5.2 普通 session 自动升级 ACP

### TC-009 普通 session 输入 `claude` 自动升级

步骤：
1. 启动普通 Zellij session（非 ACP）。
2. 在 terminal pane 输入 `claude` 并回车。

预期：
1. 若房间不存在，写入 `session_upgraded_to_acp_room`。
2. 自动注册 driver endpoint，事件包含 `driver_auto_detected`。
3. `acp-bar` 出现并显示 `D1`。

---

### TC-010 普通 session 输入 `codex` 自动升级

步骤同 TC-009，命令换成 `codex`。

预期：
1. provider=codex。
2. 其他行为同 TC-009。

---

### TC-011 普通 session 输入 `gemini` 自动升级

步骤同 TC-009，命令换成 `gemini`。

预期：
1. provider=gemini。
2. 其他行为同 TC-009。

---

### TC-012 普通 session 输入 `zellij review/reviewer/view` 触发 EnsureRoom

步骤：
1. 普通 session 内输入：`zellij reviewer 123456`（或 `zellij view events 123456`）。

预期：
1. 触发房间 EnsureRoom 初始化。
2. 不会触发 `driver_auto_detected`（因为这不是 provider 直接启动命令）。

---

### TC-013 默认 tab 自动命名

步骤：
1. tab 名为默认 `Tab #1`。
2. 在该 tab 的 pane 内触发 TC-009。

预期：
1. tab 名变成 `ACP <room_id>`。

---

### TC-014 自定义 tab 不被覆盖

步骤：
1. 将 tab 重命名为 `work`。
2. 在该 tab 的 pane 内输入 `claude` 回车。

预期：
1. tab 名保持 `work` 不变。

## 5.3 观察命令 `view`

### TC-015 `zellij view events <id>`

步骤：
1. 执行：`zellij view events <room_id>`。

预期：
1. 打印房间信息（room/session/transport/participants）。
2. 进入持续跟随模式，新增事件实时出现。

---

### TC-016 `zellij view code <id>`

步骤：
1. 执行：`zellij view code <room_id>`。

预期：
1. 打印房间信息。
2. 跟随 `code.log`（file hint / pane snapshot）。

---

### TC-017 无 ID 且不在 session 内

步骤：
1. 在普通 shell 执行：`zellij view events`。

预期：
1. 报错：请提供 shared room ID 或在 session 中执行。
2. 退出码非 0。

## 5.4 review request 流程

### TC-018 生成 review request 并落盘

步骤：
1. 在有 driver 活动的 room 执行：`zellij review request <room_id>`。

预期 stdout：
1. `Request ID: rr-xxxx`。
2. 打印 `[ACP_REVIEW_REQUEST_BEGIN] ... [ACP_REVIEW_REQUEST_END]`。
3. 打印 `Saved request JSON:` 和 `Saved request text:`。

预期文件：
1. `<ROOM_DIR>/review/requests/latest.json`。
2. `<ROOM_DIR>/review/requests/latest.txt`。
3. `<ROOM_DIR>/review/requests/history.jsonl` 追加一条。

---

### TC-019 有 live reviewer pane 时自动注入 request

步骤：
1. 确保 reviewer 已启动且绑定 pane。
2. 执行 `zellij review request <room_id>`。

预期：
1. stdout 出现 `Injected review request into reviewer ...`。
2. reviewer pane 收到 request block。
3. `events.log` 含 `review_request_injected_to_reviewer`。

---

### TC-020 无 live reviewer pane 时 waiting

步骤：
1. room 内没有 reviewer，或 reviewer pane 不在线。
2. 执行 `zellij review request <room_id>`。

预期：
1. stdout：`No live reviewer panes were found. Request kept on disk.`
2. `events.log` 有 `review_request_waiting_for_reviewer_delivery`。

---

### TC-021 session 不活跃时 request 仅落盘

步骤：
1. 先创建 room 并退出 session（保证房间缓存在，但 session 不活跃）。
2. 执行 `zellij review request <room_id>`。

预期：
1. request 文件照常生成。
2. 不发生 reviewer 注入。

---

### TC-022 backfill driver pane binding

步骤：
1. 准备场景：room 里只有 1 个 driver endpoint，且其 `bound_pane_id` 为空或过期。
2. 保持 session 活跃并只存在一个候选 driver terminal pane。
3. 执行 `zellij review request <room_id>`。

预期：
1. endpoint metadata 被回填为正确 `bound_pane_id`。
2. `events.log` 记录 `driver_pane_binding_backfilled`。

## 5.5 review feedback 流程

### TC-023 should_send=true 且 pane 可解，注入 driver

步骤：
1. 准备有效 `ACP_REVIEW_RESPONSE_V1` 文本（`should_send: true`）。
2. 执行：`zellij review feedback <room_id> --file <feedback_file>`。

预期：
1. stdout 显示 `Will send to driver: true`。
2. stdout 显示 `Injected driver envelope into: terminal_X`。
3. driver pane 收到 `[ACP_MESSAGE_BEGIN]` envelope。
4. `events.log` 包含 `review_feedback_injected_to_driver`。

---

### TC-024 should_send=true 但 session 不活跃

步骤：
1. session 退出，仅保留 room 文件。
2. 执行同 TC-023。

预期：
1. envelope 文件落盘。
2. stdout：`Room session is not active. Envelope kept on disk.`
3. `events.log` 包含 `review_feedback_waiting_for_driver_delivery`。

---

### TC-025 should_send=true 但 pane 不可解

步骤：
1. 构造活跃 session，至少两个 terminal pane，无法唯一定位目标 pane。
2. 执行反馈命令。

预期：
1. stdout：`Driver pane not resolved. Envelope kept on disk for manual delivery.`
2. `events.log` 写 `review_feedback_waiting_for_driver_delivery`，reason 为 `target_pane_not_resolved`。

---

### TC-026 should_send=false

步骤：
1. feedback block 中设置 `should_send: false`。
2. 执行反馈命令。

预期：
1. stdout：`Will send to driver: false`。
2. 不创建/更新 driver envelope 文件。
3. 不发生注入。

---

### TC-027 feedback 缺失 block marker 报错

步骤：
1. 提供不含 `[ACP_REVIEW_RESPONSE_V1]` 的文件。
2. 执行反馈命令。

预期：
1. 报错：缺少 start 或 end marker。
2. 退出码非 0。

---

### TC-028 feedback room_id 与目标 room 不一致

步骤：
1. 命令目标 room 为 `A`。
2. feedback block 里 `room_id: B`。

预期：
1. 报错 `room_id ... does not match target room ...`。
2. 不写入反馈文件。

---

### TC-029 source_endpoint override

步骤：
1. feedback block 省略 `source_endpoint`。
2. 执行：`zellij review feedback <room_id> --file <f> --source-endpoint reviewer-1234`。

预期：
1. 解析成功。
2. stdout 显示 `Source endpoint: reviewer-1234`。

## 5.6 ACP Bar 与布局可见性

### TC-030 ACP bar 全宽显示

步骤：
1. 打开 ACP 房间，终端宽度 >= 120。

预期：
1. 底部显示 `ACP` 前缀。
2. 显示 `room: <id> | self: provider/role | roles: driver(x) reviewer(y)`。

---

### TC-031 ACP bar 窄宽紧凑显示

步骤：
1. 将终端宽度缩小（例如 60）。

预期：
1. 自动降级为紧凑格式：`<id> | provider/role | Dn Rn`。

---

### TC-032 无 room_id 显示空白

步骤：
1. 在无 ACP room 上下文的场景加载 `acp-bar`。

预期：
1. bar 不显示房间文本（空白区域）。

## 5.7 文件结构与数据一致性

### TC-033 房间与 review 目录完整性

步骤：
1. 执行完整流：`claude -> reviewer -> review request -> review feedback`。
2. 检查 `<ROOM_DIR>`。

预期目录：

```text
<ROOM_DIR>/
  room.json
  events.log
  events.jsonl
  code.log
  code.jsonl
  endpoints/*.json
  review/
    requests/latest.json
    requests/latest.txt
    requests/history.jsonl
    feedback/latest.json
    feedback/latest.txt
    feedback/history.jsonl
    driver-inbox/latest.txt   # should_send=true 时出现
```

预期一致性：
1. `events.log` 与 `events.jsonl` 事件数量同趋势增长。
2. endpoint 中 `bound_pane_id` 与实际 session pane 对齐。
3. request/feedback 的 `request_id` 可互相关联。

## 6. 故障注入补充（覆盖异常分支）

以下是为了覆盖异常分支的“人工故障注入”方案。

### FI-001 reviewer 注入失败分支

方法：
1. 保持 reviewer endpoint 在 metadata 中存在，但把对应 pane 关闭。
2. 执行 `zellij review request <room_id>`。

预期：
1. `review_request_waiting_for_reviewer_delivery`。
2. reason 为 `reviewer_injection_failed` 或 `no_live_bound_reviewer_panes`。

### FI-002 driver 注入失败分支

方法：
1. 生成 should_send=true feedback。
2. 在反馈注入瞬间关闭目标 pane 或让 pane 不可用。

预期：
1. `review_feedback_waiting_for_driver_delivery`。
2. reason 为 `target_pane_not_resolved` 或 `pane_query_failed`。

## 7. 回归执行顺序（建议）

最小回归（10 分钟）：
1. TC-001
2. TC-006
3. TC-015
4. TC-018
5. TC-023
6. TC-030

标准回归（30-40 分钟）：
1. TC-001~TC-022
2. TC-023~TC-033

全量回归（含故障注入）：
1. 全部 TC
2. FI-001
3. FI-002

## 8. 执行记录模板

每次执行建议记录：

```text
Date:
Executor:
Branch/Commit:
Room ID:

Case ID:
Result: PASS / FAIL
Observed stdout:
Observed events:
Artifacts:
Notes:
```

