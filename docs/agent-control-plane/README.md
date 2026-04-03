# Agent Control Plane 设计

这份文档记录了一个基于 Zellij 构建 agent 协作工作流的设计方向。

目标不是去深度集成各种 provider 的原生桌面应用。
目标是把 Zellij 标准化成统一运行时，并把缺失的协作原语直接补进 Zellij。

## 当前 MVP 状态

仓库里已经落下了第一阶段的命令面和本地房间元数据层：

- `zellij claude [id]`
- `zellij codex [id]`
- `zellij gemini [id]`
- `zellij reviewer [id]`
- `zellij view events [id]`
- `zellij view code [id]`

当前行为：

- 不带 `id` 会创建一个新的共享房间 ID，并打印出来
- 带 `id` 时，如果房间存在就加入；不存在就创建
- 房间和参与者元数据会持久化到 Zellij 的缓存目录
- `reviewer` 会记录一份启动 bootstrap prompt 和固定的 review message template
- `view events` 和 `view code` 会跟随房间自己的流文件

这版是有意保持为基础版。
bridge 还没有真正镜像 pane 的实时输入输出，也还没有把 pane 内文件改动流接进来。
下一阶段会把这些房间流真正接到 pane I/O 上。

进度记录：

- [2026-04-03 进度](/Users/chenbiao/zellij/docs/agent-control-plane/progress/2026-04-03.md)

协议文档：

- [Reviewer 自主性与通信协议](/Users/chenbiao/zellij/docs/agent-control-plane/reviewer-protocol.md)

第一阶段目标 provider：

- Claude Code
- Codex
- Gemini CLI / Gemini Code Assist agent mode

在第一版实现里，它们都会被统一视为运行在终端 pane 里的程序。

## 为什么先做 Zellij

这个设计明确选择了 Zellij-first 路线。

原因：

- Zellij 已经掌握 pane 的输入输出
- Zellij 已经管理 PTY 和屏幕渲染
- Zellij 已经支持程序化 pane 控制
- Zellij 有源码，可以直接补运行时能力
- 一旦所有 agent 都被归一成 pane，不同 provider 之间的差异会小很多

这是一个明确的取舍。
我们不去深挖每家原生桌面应用，而是定义一个统一终端运行时，并把这个运行时做聪明。

## 产品判断

系统的核心不是 panel 布局，也不是 session 列表。
核心是 message bridge。

其他能力都建立在 bridge 之上：

- 实时事件流
- 代码可见性
- reviewer 自主性
- 人工介入
- 结对编程
- 后续调度与监督

## 第一阶段三条主线

第一个里程碑应拆成三部分：

1. `bridge`
2. `reviewer`
3. `view`

这三部分建议按这个顺序实现。

## Part 1: Bridge

bridge 是传输和路由层。

它应该实现于 Zellij 的 pane I/O 边界，或者紧贴这个边界的一层。

### 职责

- 捕获 pane 的 stdin 写入
- 捕获 pane 的 stdout / PTY 输出
- 捕获渲染后的屏幕快照
- 暴露实时事件订阅能力
- 允许程序化地向 pane 注入消息
- 保持一份可回放的本地事件日志

### 设计原则

bridge 应该是可靠的，而不是智能的。

它负责搬运和持久化消息。
除了最小限度的类型标注和路由之外，不应该替消息做太多语义判断。

### Endpoint 模型

endpoint 是任何一个连接到 bridge 的在线参与者。

例子：

- 一个 Claude Code pane
- 一个 Codex pane
- 一个 Gemini pane
- 一个 reviewer 进程
- 一个给人看的控制面板
- 一个 manager 总控面板

建议的 endpoint 结构：

```json
{
  "endpoint_id": "driver-claude-1",
  "endpoint_type": "zellij_pane",
  "provider": "claude",
  "pane_id": "terminal_7",
  "session_name": "coding-main",
  "role": "driver",
  "repo_root": "/Users/chenbiao/zellij"
}
```

### Shared ID 模型

用户侧模型必须保持足够简单。

系统不应该要求用户去记或输入运行时才生成的内部 ID。

建议的用户侧概念就是一个共享 ID。

建议术语：

- `shared_id`
- 或者直接叫 `id`

含义：

- 如果没有传 ID，就创建一个新的协作组，并生成新的 ID
- 如果传入的 ID 已经存在，就加入这个组
- 如果传入的 ID 还不存在，就由当前参与者创建它

这样可以解决“谁先启动”的问题，而且不需要额外加一个 create 命令。

### 内部 ID 与用户侧 ID

系统内部仍然应该保留更丰富的身份信息：

- `shared_id`
  用户通常唯一需要关心的 ID
- `endpoint_id`
  某个连接点的运行时身份
- `agent_id`
  可选，用于描述某个可复用 agent persona 的逻辑身份

但默认的用户交互对象，应该始终是 `shared_id`。

例子：

```json
{
  "shared_id": "482731",
  "endpoint_id": "endpoint-9b1f",
  "agent_id": "claude-driver-main",
  "role": "driver"
}
```

### 为什么 shared_id 优先

它用一条规则就解决了“谁先起”的问题：

- 不带 ID 就创建新组并打印 ID
- 带显式 ID 就走 create-or-join

这个交互很像轻量的“面对面建群”。

理想体验应该是：

- 启动一个东西
- 得到一个 ID
- 把这个 ID 告诉下一个参与者
- 对方带着这个 ID 启动后立刻加入

### 共享组里的角色

一个 shared ID 下面可以挂多个不同角色的参与者：

- `driver`
- `reviewer`
- `human`
- `viewer`
- `manager`

角色可以随着时间变化。
同一个参与者在需要时也可以同时持有多个角色。

### 推荐的组元数据

建议字段：

- `shared_id`
- `name`
- `created_by`
- `created_at`
- `shared_task_id`
- `repo_root`
- `current_driver_endpoint_id`
- `visibility`
- `transport`

### Presence 与发现

一旦参与者通过 shared ID 接入，它就应该可以被发现。

建议的 presence 载荷：

```json
{
  "shared_id": "482731",
  "endpoint_id": "endpoint-9b1f",
  "agent_id": "claude-driver-main",
  "provider": "claude",
  "role": "driver",
  "status": "online",
  "pane_id": "terminal_7"
}
```

bridge 应该能实时回答这些问题：

- 现在有哪些 shared ID
- 每个 shared group 里有哪些参与者
- 当前有哪些角色在线
- 当前哪个 endpoint 是 active driver
- 哪些 endpoint 是可写的

### Create-or-join 语义

每一个主要入口命令都应遵循同一条规则：

- 不带位置参数 ID：创建新的 shared group 并打印生成的 ID
- 带位置参数 ID：如果找到就加入，找不到就创建

这样用户模型才会足够小，足够一致。

### Endpoint 定位

大部分面向人的命令，应该接受两种目标方式：

- `shared_id`
- `endpoint_id`

经验规则：

- 基于 `shared_id` 的命令适合做加入、发现和广播
- 基于 `endpoint_id` 的命令适合做精确注入和精确控制

例子：

- 向 shared group `482731` 里的 active driver 发送一条人类消息
- 直接把一条 review comment 发给 `endpoint-9b1f`

### 消息路由规则

bridge 应该支持两种路由方式：

- 显式 endpoint
- 当前房间里的某个角色

例子：

- `target_endpoint_id = endpoint-9b1f`
- `target_role = driver in shared_id 482731`

这样 human 和 reviewer 的命令都可以保持简单。

### Channel 模型

bridge 应该先定义一小组有类型的 channel。

推荐起步 channel：

- `pane.stdin.raw`
- `pane.stdout.raw`
- `pane.screen.render`
- `pane.event`
- `human.input`
- `review.comment`
- `pane.control`

后续可以派生出更高层 channel：

- `command`
- `command_output`
- `file_change`
- `diff`
- `summary`
- `test_result`

### 消息示例

向 driver pane 注入一条人类消息：

```json
{
  "type": "human.input",
  "target_endpoint_id": "driver-claude-1",
  "text": "补一个回归测试，重点看 cursor hidden 分支",
  "send_enter": true
}
```

一个 pane 的渲染屏幕更新：

```json
{
  "type": "pane.screen.render",
  "source_endpoint_id": "driver-claude-1",
  "pane_id": "terminal_7",
  "text": "⏺ Write(zellij-server/src/tab/mod.rs)\n⏺ Bash(cargo test -p zellij-server ...)",
  "timestamp": "2026-04-03T01:00:00+08:00"
}
```

reviewer 产出的一条 review comment：

```json
{
  "type": "review.comment",
  "source_endpoint_id": "reviewer-1",
  "target_endpoint_id": "driver-claude-1",
  "severity": "medium",
  "file": "zellij-server/src/tab/mod.rs",
  "category": "missing_test",
  "text": "隐藏光标分支有逻辑改动，但还没有 IME 回归测试。"
}
```

### Capability 模型

每个 endpoint 都应该声明自己能发什么、能收什么。

例子：

- driver pane 可以发输出，也可以接收文本输入
- diff viewer 可以接收 diff 数据，但不应该接收任意聊天消息
- reviewer 可以发 review comment，也可以按需接收 control 命令

建议的初始能力：

- `can_emit_screen`
- `can_emit_stdout`
- `can_emit_events`
- `can_receive_text_input`
- `can_receive_review_comments`
- `can_receive_control`

### Transport

第一版 transport 应该保持本地、简单。

推荐的 MVP transport：

- 本地 Unix domain socket 或 loopback TCP
- JSON 载荷
- append-only 的本地日志，便于回放

这样第一版可以保持 terminal-first、machine-local。

### 面向网络的设计

第一阶段可以只做本地，但 bridge 从一开始就要为局域网协作留干净的扩展口。

目标场景：

- 两个或更多人加入同一个局域网房间
- 一个人做 driver，另一个人做 reviewer
- 每一侧背后都可以挂自己的本地 agent
- 会话过程中角色可以切换

这本质上就是通过 bridge 做结对编程。

因此 transport 必须尽早抽象出来。

建议的 transport 演进路径：

1. 本地 Unix socket
2. loopback TCP
3. 局域网 WebSocket room transport
4. 可选的 raw TCP transport，用于更低开销的部署

bridge API 不应该假设自己永远只跑在本机。
从第一天开始，endpoint 就应该能携带 host 或 peer 元数据。

网络协作模式下，首选 transport：

- WebSocket

原因：

- 客户端接入更简单
- 后续更容易支持浏览器和桌面端
- 更容易表达 room 和 presence 语义
- 开发阶段更容易观察消息

可选的后续优化路径：

- raw TCP

原因：

- 协议开销更低
- 适合高频、本地网络的特殊部署场景
- 如果 framing 明确，可以复用同一套 message schema

为后续网络化建议保留的 room 概念：

- `room_id`
- `peer_id`
- `role`
- `presence`
- `current_target`
- `shared_task_id`

这样后面要做局域网模式时，不需要重做消息模型。

### 当前可直接利用的 Zellij Hook

即使还没有更深的源码改动，Zellij 现在已经提供了一些足够有用的控制原语：

- 向 pane 写入字符
- 向 pane 发送按键
- dump 当前 pane 的屏幕
- 订阅 pane 的 render 更新

这些原语已经足够先验证 bridge 概念，再决定是否深入改 PTY 路径。

### 建议的命令面

命令模型应该尽量简单、自然。

用户在常规路径里不应该还要显式先执行一次 “create room”。

核心规则：

- 启动时不带 ID -> 创建新 shared group 并打印生成的 ID
- 启动时带 ID -> 找到就加入，找不到就创建

这条规则应该对 driver 和 reviewer 入口都一致生效。

#### 建议命令

Driver 入口：

- `zellij claude [id]`
- `zellij codex [id]`
- `zellij gemini [id]`

例子：

- `zellij claude`
  创建新的 shared group，并打印一个新 ID
- `zellij claude 482731`
  加入或创建 shared group `482731`
- `zellij codex 482731`
  加入或创建 shared group `482731`

Reviewer 入口：

- `zellij reviewer [id]`

例子：

- `zellij reviewer`
  创建一个新 shared group，并在里面启动 reviewer
- `zellij reviewer 482731`
  加入 shared group `482731`，并在其中开始 review

观察入口：

- `zellij view events [id]`
- `zellij view code [id]`
- 后续 `zellij view board [id]`

例子：

- `zellij view events 482731`
- `zellij view code 482731`

#### 可选的显式 bridge 命令

这些命令仍然适合调试和高级控制，但不应该成为主交互路径：

- `zellij bridge list`
- `zellij bridge who 482731`
- `zellij bridge send 482731 --role driver --text "补一个回归测试"`
- `zellij bridge send --endpoint endpoint-9b1f --text "先跑测试再继续"`

### Reviewer 命令形态

reviewer 应该是一个一等命令，而不是内部硬编码出来的特殊进程。

建议入口：

- `zellij reviewer [id]`

建议选项：

- `--watch-role driver`
- `--mode observe|suggest|auto-review|gatekeeper`
- `--provider claude|codex|gemini`
- `--send-direct`
- `--needs-human-approval`

例子：

- `zellij reviewer 482731 --watch-role driver --mode suggest`
- `zellij reviewer 482731 --watch-role driver --mode auto-review`

reviewer 自身也应该注册成 room 里的一个 endpoint。

这意味着：

- 它会拥有自己的 `endpoint_id`
- 它可以接收 control 消息
- 其他 reviewer 或 human 也可以给它发消息

这样 reviewer 才是可组合的，而不是特殊分支逻辑。

### View 命令形态

建议的 view 命令：

- `zellij view events [id]`
- `zellij view code [id]`
- 后续 `zellij view board [id]`

这些 view 应该先解析 shared ID，再订阅对应 endpoint 的相关流。

## Part 2: Reviewer

真正需要额外自治能力的，不是 coding agent，而是 reviewer 或 scheduler。

reviewer 是一个 sidecar 进程，它观察一个或多个 driver endpoint，并决定自己是否应该发声。

### 职责

- 观察 driver 最近的活动
- 查看改动过的文件和当前 diff
- 发现可能的问题
- 生成 review comment
- 按需把 comment 再发回给 driver

### 推荐模式

reviewer 应该支持多个运行模式：

- `observe`
- `suggest`
- `auto-review`
- `gatekeeper`

建议的 MVP 默认模式：

- `suggest`

在 `suggest` 模式下，reviewer 不会直接打断 driver。
它产出 comment，由 human 决定批准还是拒绝。

### 自主性要求

reviewer 的自主性必须被约束。
否则它会变得很吵，最后没有价值。

reviewer 不应该对每一个事件都立即响应。

建议策略：

- 只有在短暂静默后才考虑发声
- 遇到强信号时可以立即发声：
  - 测试失败
  - 命令反复失败
  - 大而危险的 diff
  - 同一区域反复编辑但没有实质进展
- 对重复 comment 做去重
- 按文件或问题类别设置 cooldown

### Reviewer 状态机

建议状态：

- `idle`
- `observing`
- `thinking`
- `pending_human_approval`
- `sent`
- `cooldown`

### 输入

reviewer 应该订阅这些输入：

- 渲染后的 pane 输出
- 最近的 pane 事件流
- 当前 repo diff
- 最近触碰的文件
- 最新测试结果

### 输出

reviewer 的输出应该是结构化的，而不只是自由文本。

建议字段：

- 严重级别
- 目标文件
- 问题类别
- 简短反馈
- 建议的下一步动作

这种结构更方便渲染、路由和去重。

## Part 3: View

人类需要一层专门的观察界面。

一个 panel 不够，因为要回答的是两个不同问题：

- agent 现在在做什么
- 代码到底改了什么

这两件事应该被拆成不同视图。

### View 1: Events

目的：

- 显示实时活动
- 显示当前命令
- 显示最近文件动作
- 显示失败和告警

这是操作视图。

建议命令：

- `zellij view events`

### View 2: Code

目的：

- 显示最近触碰的文件
- 显示当前 diff
- 显示改动文件列表
- 显示带变更标记的当前文件

这是代码可见性视图。

建议命令：

- `zellij view code`

### 后续可选视图：Board

目的：

- 同时监督多个 driver 和 reviewer
- 显示 blocked 状态
- 显示待处理 review
- 显示重叠冲突

后续建议命令：

- `zellij view board`

## 结对编程模型

这个设计天然支持 driver / reviewer / human 的工作流。

角色：

- `driver`：主要负责写代码的 agent
- `reviewer`：观察并提出批评意见的 sidecar agent
- `human`：最终拍板的人

流程：

1. Human 把需求交给 driver。
2. Driver 在 pane 里工作。
3. Bridge 捕获 driver 的活动。
4. Reviewer 观察 bridge 并生成反馈。
5. Human 决定批准、修改还是拒绝这些反馈。
6. 被批准的反馈再通过 bridge 发回 driver pane。
7. Driver 继续迭代。

这是建立在 Zellij 之上的第一种真正有意义的 pair programming 形态。

## 建议命名

命名保持短、直白：

- `bridge`
- `reviewer`
- `view events`
- `view code`
- 后续 `view board`

命名应该直接反映职责，不要绕。

## MVP 顺序

第一个里程碑不要试图一口气解决所有事。

建议实现顺序：

1. 先做 bridge 的注册、订阅、快照、发送原语
2. 本地持久化 bridge 事件，支持回放
3. 做 `view events`
4. 做 `view code`
5. 做 `suggest` 模式下的 reviewer
6. 加上带人工批准的 reviewer-to-driver 注入

## 第一版非目标

第一版不应该尝试做这些：

- 深度集成每一家 provider 的原生桌面应用
- 跨机器调度
- reviewer 的全自动接管
- 多任务高级调度
- Web UI

这些都可以等本地 Zellij 流程被证明有价值之后再往上加。

## 未来扩展：局域网结对编程

bridge 未来应该支持一个轻量的局域网模式。

目标：

- 一个同事可以 drive
- 另一个同事可以 review
- 双方都可以在自己角色背后挂本地 agent
- 人机协作闭环可以跨机器，而不只是跨 pane

这应该被看成 transport 扩展，而不是产品模型重做。

仍然适用的核心原语不变：

- bridge
- reviewer
- view
- driver / reviewer / human 角色

变化的只有 transport，从 purely local 变成 network-visible。

到了这个阶段，首选 transport 应该是 WebSocket。
raw TCP 作为优化路径保留，但不应作为默认方案。

## 总结

这个系统第一版真正有用的形态应该是：

- Zellij-first
- bridge-centered
- terminal-local
- human-in-the-loop
- reviewer-assisted

bridge 是底层。
reviewer 是第一个有智能的 sidecar。
view 是第一层面向人的产品界面。
