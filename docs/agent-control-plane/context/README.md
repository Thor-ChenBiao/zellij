# Agent Control Plane 上下文导读

这份目录给后续 session 用。

如果要快速接手这条线，建议按下面顺序读：

1. [总体设计](/Users/chenbiao/zellij/docs/agent-control-plane/README.md)
2. [进度记录](/Users/chenbiao/zellij/docs/agent-control-plane/progress/2026-04-03.md)
3. [Reviewer 协议](/Users/chenbiao/zellij/docs/agent-control-plane/reviewer-protocol.md)
4. [原始设计笔记](/Users/chenbiao/zellij/docs/agent-control-plane/notes/agent-control-plane-ideas.md)
5. [Codex 会话历史快照](/Users/chenbiao/zellij/docs/agent-control-plane/context/codex-history-019d4ea6-62ec-77c1-bff7-f013e8d17116.jsonl)
6. [本地测试记录](/Users/chenbiao/zellij/test-results/00_test_report.md)

## 这些文件分别是什么

[README.md](/Users/chenbiao/zellij/docs/agent-control-plane/README.md)
- 当前 ACP 方案的主设计文档
- 包含命令模型、room 模型、bridge / reviewer / view 的总体设计
- 也包含目前已经落地的 MVP 状态说明

[2026-04-03.md](/Users/chenbiao/zellij/docs/agent-control-plane/progress/2026-04-03.md)
- 这条线做到哪一步的进度记录
- 哪些已经做了，哪些还没做
- 适合快速了解当前工程状态

[reviewer-protocol.md](/Users/chenbiao/zellij/docs/agent-control-plane/reviewer-protocol.md)
- reviewer 的身份、消息格式、自主性和回注协议
- 如果后面继续做 reviewer 自动化，这份文档是关键入口

[agent-control-plane-ideas.md](/Users/chenbiao/zellij/docs/agent-control-plane/notes/agent-control-plane-ideas.md)
- 最早一轮的原始产品思考
- 包括 message bridge、pair programming、human-in-the-loop、provider 适配这些讨论
- 偏“为什么这么设计”

[codex-history-019d4ea6-62ec-77c1-bff7-f013e8d17116.jsonl](/Users/chenbiao/zellij/docs/agent-control-plane/context/codex-history-019d4ea6-62ec-77c1-bff7-f013e8d17116.jsonl)
- 这次 Codex 会话的原始历史快照
- 主要保留了用户侧的连续需求和过程上下文
- 适合追溯“这个需求是怎么演化到现在的”

[00_test_report.md](/Users/chenbiao/zellij/test-results/00_test_report.md)
- 这条线的本地测试记录
- 适合用来复现和做回归检查

## 读取建议

如果是新 session，推荐用下面的方式接手：

1. 先读 [README.md](/Users/chenbiao/zellij/docs/agent-control-plane/README.md)，建立整体模型
2. 再读 [2026-04-03.md](/Users/chenbiao/zellij/docs/agent-control-plane/progress/2026-04-03.md)，确认当前实现状态
3. 如果要继续做 reviewer，接着读 [reviewer-protocol.md](/Users/chenbiao/zellij/docs/agent-control-plane/reviewer-protocol.md)
4. 如果要理解需求来源和设计动机，再看 [agent-control-plane-ideas.md](/Users/chenbiao/zellij/docs/agent-control-plane/notes/agent-control-plane-ideas.md)
5. 如果要看最原始的连续聊天上下文，再看 [codex-history-019d4ea6-62ec-77c1-bff7-f013e8d17116.jsonl](/Users/chenbiao/zellij/docs/agent-control-plane/context/codex-history-019d4ea6-62ec-77c1-bff7-f013e8d17116.jsonl)

## 当前说明

这次相关上下文已经放进仓库，但不是把整个 `~/.codex/` 原样拷进去。

当前入库的是：
- 可读的设计文档
- 进度记录
- reviewer 协议
- 原始设计笔记
- 本次相关的 Codex 会话历史快照
- 本地测试记录

这样做的目的，是让后续 session 既能看到“整理后的结论”，也能回到“原始上下文”。
