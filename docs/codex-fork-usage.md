# Codex fork/父子线程规则

## 公开源码依据

固定官方 revision：openai/codex 的 092d3acd6bec3e3a14bdc7e7a2810ab628ab759d。已核查 Apache-2.0 许可证；本项目未复制或链接其实现。

- [历史筛选](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/src/agent/control/spawn.rs#L127)：子线程继承上下文，不继承父线程 TokenUsageRecord。
- [paginated 筛选](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/src/agent/control/spawn.rs#L1208)：该模式还移除继承 TokenCount 和压缩检查点中的 usage；其他模式不能据此假设一定已清理。
- [fork 测试](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/src/agent/control_tests.rs#L2344)：验证父 response 记录不保留，子线程累计从自己的 usage 开始。只审阅该测试，未执行整个官方工程。

source.thread_spawn.parent_thread_id 也可能只是父子关系，并不一定表示复制过父历史。不能据模型、昵称、路径或标记是否存在直接判断历史账单归属。

## 当前实现（p2a-5）

- forked_from_id 或 nested parent 提示出现时，要求非空 owner ID。两种 parent 提示同时存在必须一致。
- 首个子线程 metadata 锁定 owner。嵌入父/祖先 metadata 必须沿已声明链出现，不能替换 owner。改写 parent、环路、未声明线程和回到祖先历史均失败。
- 现代 response 必须匹配 owner。外来 response 即使声称来自父线程也不会自动计入或丢弃；无法解释时失败。
- legacy fork 采用已审查 tokscale-core 的 UUIDv7 时间部分边界思路，并严格要求 turn 的毫秒部分晚于 child 创建部分。随机尾、同毫秒、未知 ID 均不能证明子线程归属。
- 较早或不明 legacy 前缀验证数字后列入 unclassified_fork_prefix_records，保留累计基线但不计入子线程可信 token。进入子线程 turn 后再退回不明/较早 turn 则失败。
- 整个文件既无可确认的自己的 legacy turn，又无自己的 response，只存在未归属用量时，返回 unsupported_legacy_fork，不能发布成功零用量。
- parser 缓存只保留哈希身份与允许的投影，不包含原始 session/thread ID。暖扫不清除 coverage 缺口。

这些规则是保守适配，不是对所有 fork 历史形态的完整支持。未归属前缀始终使 coverage_complete=false。

## 验证

实际实现测试在 tests/codex_records.rs，覆盖 owner 锁定、祖先链/环/冲突、现代身份、legacy 严格边界、同毫秒拒绝、非法计数、父子独立计数及缓存 coverage。

可选本地限定范围对拍：

    cargo build --locked --examples
    python3 scripts/compare-local.py codex-jsonl PRIVATE_CODEX_ROOT --fork-only --limit 100

matched、errors、changed、oracle_unqualified 必须分开解释。不要把私有会话、路径或对拍输出直接附到公开 issue。
