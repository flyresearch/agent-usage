# 统计契约与适配注意事项

这是实现约束。P2a 的可执行结构已固定在 `src/protocol.rs`；字段、错误和缓存语义见 [协议 v1](protocol-v1.md)，测试调用实际 parser 和 CLI。

## 规范化口径

使用互斥 token bucket：非缓存 input、cache_read、cache_write、非 reasoning output、reasoning。只有来源明确提供 reasoning 子集时才拆分；未提供时保留在 output，不能猜测。父 bucket 的细分不能再次计入总数。负数、非整数、子集超过父计数必须诊断。

- **Codex**：input 包含 cache_read 与 cache_write，output 包含 reasoning；新格式 token_usage_record 按 response 身份计入实际 usage，token_count 仅作镜像/上下文估算检查；旧格式按累计差分与重放规则处理，不能累加每个快照。计数回退需显式策略。total-only carry 单列未分类诊断，不补到可计费分项。
- **Claude Code**：input、cache read、cache creation 独立；cache TTL 是 creation 子集。p2a-5 对已识别的 TTL>creation 矛盾隔离该消息的 cache-write、保留其他独立分项，并标 invalid_cache_ttl_records；输出为已知用量，不把有争议的分项声称为真实零。同一 message/request 流式更新不能反复累加；P1 的字段 max 参考例不等于已确定所有异常更新策略。
- **agy**：gen_metadata 的 chatModel #1 → usage #4；input=#1+#2、cache_read=#5、output=#9、reasoning=#10，#3 是输出总量。时间从 native timestamp 或 steps 关联取得；不能用文件 mtime。protobuf 映射为逆向观察，应对未知 schema 提示。
- **Hermes**：以 model usage 表为主体，含辅助调用；sessions 仅补主调用未覆盖残差，负残差须诊断。output 包含 reasoning，input 不含 cache。跨日聚合无法重建日分配时保留 interval 和 unallocated。
- **OpenClaw**：支持 SQLite 普通 JSON 和 Zstd。专属 Codex HOME 是 Codex harness 主来源；主库/runtime 对应调用仅核对。明确的非 Codex 来源独立计入；无法确认的历史来源单列 unclassified。归属依赖可靠 linkage/明确配置，不凭模型名或当前窗口标签判断历史。

## 快照与完整性

信封包含 schema/collector/parser 版本、host_id、配置指纹、生成时间、时区和 complete。聚合按 agent/model/day/受控来源标签；统一 Asia/Singapore，缺失或无法精确分配的日期使用未分摊桶并给 time_basis。

coverage 区分可信 token、无法分配日期、未知总量基数、来源未分类和不支持字段。complete 表示启用来源读取成功，不等于历史覆盖完整；UI 必须保留这一区别。

只输出允许的数字和受控标签，不输出原始 session/thread ID、用户路径、正文、工具参数或邮箱。SSH 主机与路径配置留在本地私有文件，示例使用虚构值。

中心只接受当前配置版本的完整结果，原子替换该 host last-good，然后跨机求和。失败保留旧快照；未采集机器不是零。首次失败、过期结果和配置变更必须有明确状态。

## 已知实现风险

P1 的九项 Python 检查只验证参考规则，未运行上游 Rust parser。Windows 原生运行、SQLite 链接版本、OpenClaw 同调用归属及暖扫描性能仍需实测。不得宣称全量验收通过。

## Observed streaming refinements (p2b-4)

- Codex: an exact `total == last` reset is accepted at a previously unseen explicit task ID only when the matching turn context differs from the preceding usage turn. Replayed/mismatched task IDs do not authorize small regressions. Resets remain diagnosed.
- Claude: an initial zero-output message without cache fields can precede a terminal message that partitions the same combined input into input/cache-read/cache-write. Only matching message identity, explicit terminal/cache fields and exact input conservation authorize replacement in either arrival order. Unknown or contradictory updates still fail; unresolved provisional input is unclassified rather than presented as noncached input.
- A regular JSONL symlink whose canonical target is already inside the same configured root is an excluded duplicate alias. Directory, dangling, out-of-scope and other-format links remain rejected. The exclusion has an explicit `deduplicated_symlink_files` count.

## 已计量响应/API 调用计数（p2b-5）

每行新增 `messages: {known, complete}`。这是已识别用量对应的助手响应/API 调用数，不是用户消息数、工具消息数、日志行数或对话轮数。去重、镜像剔除和流式归并先于计数汇总；计数相加检查溢出。

- 新版 Codex 按 response 身份计一次；旧累计 token_count 无可靠响应数，保留未知。
- Claude 按已归并的 message/request 身份计一次，未完成的 provisional 用量不计。
- agy 有 response 身份时计一次；无身份 generation 保留未知。
- Hermes 优先读取可选 `session_model_usage.api_call_count`，缺字段/null 时未知，非法负数失败。会话残差仍未知，不把聚合行数当调用数。
- OpenClaw 已认可的非 Codex assistant 用量事件按原去重身份计一次；runtime/mirror 不计，专属 Codex 来源遵循 Codex 规则。

同一汇总行可同时包含已知计数与未知部分：known 相加，complete 为各部分的逻辑与。旧快照缺字段时等价于 `{known:0, complete:false}`，不是已验证的真实零。计数覆盖仅限已识别用量，不声明全部历史对话完整。
