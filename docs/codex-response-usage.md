# Codex response usage 调查与适配

本文说明 Codex response 观测记录的数据契约与当前适配规则。原始日志、会话标识和实机配置不属于公开材料。

## 固定官方源码证据

来源为 `openai/codex` 的 `rust-v0.162.1`，已解析到固定 commit `092d3acd6bec3e3a14bdc7e7a2810ab628ab759d`，并核查 Apache-2.0 LICENSE。本轮只依据数据结构和调用链写适配，没有复制/链接上游实现。

- [protocol.rs](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/protocol/src/protocol.rs#L2291)：TokenUsageRecord 包含单次 usage、turn/thread 累计值及 thread/turn/session/root-turn/response 身份。它描述已完成 response 的 best-effort 观测，不能当成官方账单。
- [session/mod.rs](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/src/session/mod.rs#L4720)：record_observed_response_completed 持久化 response usage；record_token_usage_info 另行更新上下文 token 信息。
- [session/turn.rs](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/src/session/turn.rs#L2969)：同一 completed 分支先写 response 记录，再更新 token 信息。因此两个表示不能直接相加。
- [state/session.rs](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/src/state/session.rs#L216)：turn/thread 累计分别从最近 response 记录延续；它们不同于上下文窗口 token_count 的累计口径。
- [compact_remote_v2.rs](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/src/compact_remote_v2.rs#L451) 与 [compact.rs](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/src/compact.rs#L832)：压缩 response 也写观测记录。
- [recompute_token_usage](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/src/session/mod.rs#L4802)：上下文重估可产生分项为零、total 非零的 last。该值不能冒充新增模型调用。
- [官方源码测试](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/core/tests/suite/token_usage_rollout.rs)：覆盖多 response、跨 turn 和恢复后的 response 累计。仅阅读此测试，未运行整个官方工程。

## 当前选择规则（p2a-5）

1. 文件尚未出现 response 记录时采用 legacy total/last 规则。出现首条 response 后，以 usage 为新增用量，不相加 turn/thread 累计。
2. response 身份由 thread_id + response_id 组成，持久化只保留哈希；相同数值的不同 response 仍分别计入。缺身份、重复身份数据矛盾或线程冲突均失败。
3. token_count 只作镜像核对。compacted 之后的分项零、total 非零 last 可作为上下文估算，不作为新增用量。无法匹配的非零用量或缺少必要字段明确失败。
4. 已完成 response 的记录可先于镜像写入，不能因为镜像尚未出现就丢弃该 response。未完成 JSONL 末行仍延期。
5. 模型仅在 record.turn_id 与当前 turn_context 匹配时采用上下文模型，否则 unknown；压缩涉及的最新 response 不假定使用会话模型。缺时间仍未分摊。
6. 同 source 的相同 thread 同时存在 legacy-only 和 response 格式文件时，无法证明跨格式去重则失败。普通 response 文件复制按身份去重；单文件有序升级前缀允许。
7. cache_read 和 cache_write 均为 input 子集，非缓存 input 扣除二者，子集和超出父计数失败。reasoning 为 output 子集。见 [官方 cache-write 测试](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/codex-api/src/sse/responses.rs#L849)。
8. response reported total 不等于已知 input+output 时，明确报告 unclassified_response_total；不把差额补入可信分项。

coverage 包含 response_usage_records、mirrored_usage_records、context_estimate_records。usage_records 是观察到的 usage 行数，包含镜像，不能解释为调用次数。parser 版本改变时缓存失效。

## 验证

人工 fixture 与真实实现测试位于 tests/codex_records.rs。包括身份去重、镜像、压缩估算、EOF、旧格式前缀、非法计数、thread 隔离及跨文件缓存。运行命令与平台边界见 [validation](minimal-validation.md)。公开文档不附私有会话统计或原始数据。
