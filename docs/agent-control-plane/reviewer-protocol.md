# Reviewer 自主性与 Driver 通信协议

这份文档定义 reviewer 应该如何工作、它如何知道自己是谁，以及它如何通过 bridge 和 driver agent 通信。

目标是让 reviewer 的行为可预测、可组合，而不是靠“感觉”运作。

## 核心决策

- reviewer 是 shared room 里的一个一等 endpoint
- bridge 是身份、路由和消息类型的唯一可信来源
- reviewer 不应该自己去发现 pane ID 后直接和 driver 通信
- reviewer 应该通过 bridge，用 room 级消息和 driver 通信
- driver 必须始终能够明确知道，一条消息究竟来自 reviewer 还是来自 human

## Actor 模型

系统里有三个明确角色：

- `driver`
  正在写代码的 agent
- `reviewer`
  观察 driver 并产出 review 反馈的 sidecar agent
- `human`
  可以暂停、批准、改写、覆盖前两者的人类操作员

bridge 位于中间，负责四件事：

- 捕获活动
- 组装 review 上下文
- 路由 reviewer 输出
- 保留人工介入点

## Reviewer 如何知道自己是谁

reviewer 的身份不应该只靠 prompt 里的自然语言暗示。
它应该在启动时由 bridge 明确设定。

启动流程：

1. `zellij reviewer [id]` 创建或加入一个 shared room
2. bridge 注册一个新的 endpoint，带上：
   - `room_id`
   - `endpoint_id`
   - `provider`
   - `role=reviewer`
   - `target_role=driver`
3. bridge 生成一份 reviewer bootstrap packet
4. 这份 bootstrap packet 作为第一条消息注入 reviewer pane
5. reviewer 在收到新的 bootstrap packet 之前，都应把后续 review 请求视为自己这个角色的工作

bridge 还应该把这份 bootstrap payload 持久化到房间元数据里，这样 reviewer 重连时也能恢复同一身份。

## Reviewer Bootstrap 契约

这份 bootstrap packet 应该告诉 reviewer：

- 它是谁
- 它属于哪个房间
- 它要 review 谁
- 什么样的反馈算“好反馈”
- 它必须用什么格式输出
- 反馈注入前是否必须经过 human 批准

建议注入给 reviewer 的 bootstrap packet：

```text
[ACP_BOOTSTRAP_BEGIN]
room_id: 482731
endpoint_id: reviewer-7c31
role: reviewer
provider: claude
target_role: driver
mode: approval
human_override: enabled
message_schema: ACP_REVIEW_RESPONSE_V1

你是这个共享编码房间的 reviewer。
你的职责是审查 driver 当前的工作，重点关注正确性、行为回归、缺失测试、风险假设，以及没有真正收口的问题。

Rules:
- 除非被显式要求，否则不要开始实现代码
- 不要对每一个事件都立刻响应
- 等待 review 窗口出现
- 优先给出简洁、可执行的反馈
- 输出必须严格遵守指定的 review response schema
[ACP_BOOTSTRAP_END]
```

这份 packet 就是“reviewer 怎么知道自己是 reviewer”的标准答案。

## Driver 如何知道一条消息来自 Reviewer

driver 不应该收到模糊的自由文本。
每一条注入的 review 都必须带一个稳定的 bridge envelope。

建议注入给 driver 的格式：

```text
[ACP_MESSAGE_BEGIN]
room_id: 482731
message_type: review_feedback
source_role: reviewer
source_endpoint: reviewer-7c31
target_role: driver
sequence: 14
delivery_mode: approval
severity: medium
summary: 缺少 hidden-cursor IME 行为的回归覆盖
[ACP_MESSAGE_END]

[Review from reviewer-7c31]
- 为 hidden-cursor IME anchor 路径补一条回归测试。
- 验证 host cursor move 不会被重复发送。
- 修改后重新跑相关 focused test。
```

这样 driver 就能清楚看到身份来源和消息边界。

## Reviewer 和 Driver 默认不应该自由发挥地直接聊天

bridge 应该先标准化两类消息：

- `review_request`
- `review_feedback`

内部可以把它们存成 JSON。
真正注入 pane 时，再渲染成人能读的文本 envelope。

这样系统就有两层：

- 机器层：负责路由、重放、状态管理
- agent 层：负责消费文本、继续对话

## Review Request 协议

bridge 不应该每次都把整段原始 transcript 一股脑丢给 reviewer。
它应该构建一个有边界的 review bundle。

建议字段：

- room 元数据
- driver endpoint 身份
- 改动文件
- 最近命令
- 最近命令结果
- 最近相关输出摘要
- 触发 review 的原因
- 当前运行模式

建议发给 reviewer 的 request packet：

```text
[ACP_REVIEW_REQUEST_BEGIN]
room_id: 482731
request_id: rr-0014
driver_endpoint: claude-driver-9b1f
target_role: driver
mode: approval
reason: driver_turn_closed

changed_files:
- zellij-server/src/tab/mod.rs
- zellij-server/src/output/mod.rs

last_commands:
- cargo test hidden_cursor -- --nocapture

recent_output:
Driver 更新了 IME cursor 处理逻辑，并重新跑了 focused tests。

task:
Review 最新的 driver 改动。
只返回一个 ACP_REVIEW_RESPONSE_V1 block。
[ACP_REVIEW_REQUEST_END]
```

## Review Response 协议

reviewer 的输出必须是可解析、稳定的。
它不应该只是开放式长文。

建议响应格式：

```text
[ACP_REVIEW_RESPONSE_V1]
room_id: 482731
request_id: rr-0014
source_endpoint: reviewer-7c31
target_role: driver
severity: medium
confidence: high
should_send: true
summary: 改动方向是对的，但仍然缺少回归覆盖。

findings:
- 当前没有 focused regression test 覆盖 hidden-cursor 分支。

actions:
- 为 hidden-cursor IME 路径补一条回归测试。
- 确认 host cursor update 只会在 anchor 变化时发送。

notes:
- 当前实现改动面较小，而且比较局部。
[/ACP_REVIEW_RESPONSE_V1]
```

这样 bridge 才能做确定性的解析和路由。

## Reviewer 自主性模型

reviewer 应该有自主性，但必须是有边界的。

它不应该每敲几个字就插一句话。

### Reviewer 状态

- `bootstrapping`
- `observing`
- `collecting_context`
- `reasoning`
- `ready_to_send`
- `waiting_human_approval`
- `cooldown`
- `paused`

### 各状态含义

- `bootstrapping`
  reviewer 还没有收到角色契约
- `observing`
  reviewer 正在订阅 driver 活动，但还没有准备发 review
- `collecting_context`
  bridge 认为 review 窗口已经出现，正在组装证据
- `reasoning`
  reviewer 正在准备结构化响应
- `ready_to_send`
  reviewer 已经产出一份合法 review packet
- `waiting_human_approval`
  review packet 已经生成，但还没注入给 driver
- `cooldown`
  reviewer 刚发送过反馈，短时间内不应继续刷屏
- `paused`
  human 介入后暂停了自动路由

### Review 触发条件

reviewer 应该只在有限触发器上被唤醒：

- driver 看起来完成了一轮输出
- 命令执行失败
- 测试执行结束
- 高风险文件被修改
- human 显式要求 review
- 一波活动结束后，bridge 的 idle timer 到期

### 建议的 turn-close 启发式

MVP 可以先用一个简单启发式：

- 已经观察到 driver 输出
- `N` 秒内没有新的 driver 输出
- 没有仍在运行的活跃命令

第一版可以先把 `N` 设在 `2-4` 秒之间。

后面再结合更丰富的 pane / PTY 状态做增强。

## 防刷屏规则

reviewer 应该遵守这些限制：

- 每个 driver turn 最多保留一条待处理 review packet
- 和上一条已发送 review 相似的内容要去重
- cooldown 期间抑制低价值 comment
- 如果严重级别很高，允许绕过 cooldown

这是让自主性变得有价值而不是吵闹的关键。

## Human 介入模型

在控制层级上，human 必须高于 reviewer。

默认运行模式：

- `manual`
  reviewer 只起草，不自动注入
- `approval`
  reviewer 起草后等待 human 批准
- `auto`
  reviewer 起草后自动注入，除非当前已暂停

建议默认模式：

- `approval`

### Esc 语义

`Esc` 不应该被解释成 “杀掉 reviewer”。
它应该被解释成 “让这个房间进入 manual hold”。

进入 manual hold 后的效果：

- 停止 reviewer -> driver 的自动注入
- 继续捕获事件
- 可以按需继续让 reviewer 起草 packet
- 必须显式由 human 恢复后，自动路由才会继续

这样 human takeover 的语义才稳定。

## 路由模型

reviewer 不应该自己去猜目标 pane。
目标解析应该由 bridge 完成。

建议路由规则：

- reviewer 发出 `target_role=driver`
- bridge 解析出这个房间里的 active driver endpoint
- bridge 再把消息注入那个 driver endpoint

如果后面一个房间里同时存在多个 driver，那么 bridge 可以要求：

- 显式 `target_endpoint`
- 或者显式指定 `active_driver_endpoint`

但第一版里，`target_role=driver` 已经足够。

## 身份与信任边界

bridge 才是消息元数据的权威来源。
agent 本身不是。

这意味着：

- endpoint ID 由 bridge 分配
- 消息的来源和去向由 bridge 盖章
- sequence number 由 bridge 维护
- 是否允许消息被真正注入，也由 bridge 决定

agent 只负责消费渲染后的 envelope。

这样可以避免房间里出现“伪装 reviewer”的歧义。

## MVP 实现顺序

reviewer 自主性应该按这个顺序实现：

1. reviewer 启动时注入 bootstrap packet
2. 定义 `ACP_REVIEW_REQUEST` 和 `ACP_REVIEW_RESPONSE` 的解析
3. 检测基础版 driver turn-close 窗口
4. 把 review request 真正送到 reviewer
5. 在 approval 模式下缓冲 reviewer 输出
6. 把已批准的 reviewer 反馈重新注入给 driver
7. 增加 pause / resume 语义

## 实际效果

一旦这套协议存在，交互闭环就会变成：

1. driver 写代码
2. bridge 捕获 driver 的证据
3. bridge 判断 review 窗口已经出现
4. reviewer 收到结构化 review request
5. reviewer 产出结构化 review response
6. bridge 选择缓冲或直接注入
7. driver 收到一条身份明确的 reviewer 消息
8. human 可以在任何时刻打断

这就是最简单可用的 pair programming 版本：

- 一个 coding agent
- 一个 reviewing agent
- 一个 supervising human
