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
