# 人工构造契约样例

[cases.json](cases.json) 的值与标识全部人工构造，不含真实日志。这些是语义样例，不冒充上游 JSONL/数据库原始格式，也不构成可运行测试。

P2a 已在 `tests/collector.rs` 中用人工值构造 Codex/Claude 的实际 JSONL 结构，并调用本仓 Rust adapter 验证。语义样例中的 Hermes/OpenClaw/host-replacement 属于 P2b/P3，未作为已通过项。另须补齐损坏/不完整 JSONL、累计回退、跨日切分、活跃 WAL、agy protobuf、Zstd 边界、OpenClaw linkage 与 Windows 路径测试。
