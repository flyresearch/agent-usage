# P2a 解析依赖审查

审查基线：[`Nanako0129/tokscale-core`](https://github.com/Nanako0129/tokscale-core/tree/fcb85923e5544488f98001cfa390740d0009b9da)，固定 revision `fcb85923e5544488f98001cfa390740d0009b9da`。已读取该 revision 的 `Cargo.toml`、`LICENSE`、`UPSTREAM.md`、`src/sessions/codex.rs` 和 `src/sessions/claudecode.rs`。

许可证为 MIT，版权 Junho Yeo；归属全文保存在 [third_party/tokscale-core/LICENSE](../third_party/tokscale-core/LICENSE)。本仓 parser 的计数选择参考并适配了该版本的 Codex total/last 状态分支及 Claude message/request 更新规则，保留归属；未复制私有调查探针。

## 选择

不将整个 tokscale-core 作为生产依赖。本轮是经过审查的窄范围规则适配，不是完整引擎 fork，也不承诺兼容所有上游格式。原因：

- 原 crate 没有只编译这两个 parser 的 feature 边界，直接依赖还会链接 reqwest、tokio、rusqlite、zstd 和 pricing/provider 模块。
- 文件 parser 返回消息数组，部分 IO/JSON 错误被跳过；时间可退回 mtime，与本仓失败及时间契约不兼容。
- scanner 会涉及额外目录/sidecar 和共享缓存；本仓要求仅扫描显式来源，持久化投影不能含原始身份或内容。

## 明确差异

| 情况 | 固定上游行为 | 本轮行为 |
|---|---|---|
| total + last | 使用 last，total 用于去重与基线 | 相同主规则；大幅回退计入 counter_resets，小幅含混回退报错 |
| 只有 total | 正常差分；回退时换基线并跳过 | 正常差分；回退拒绝整轮 |
| 非法子集/负数 | 多处 clamp | 拒绝整轮 |
| total-only carry | 不进入互斥分项 | 单列 coverage，不补入分项 |
| Claude streaming | 逐字段 max | 可比较的递增/旧版本取较大值；交叉变化拒绝，避免拼接不存在的账单 |
| 缺时间 | 可能使用 mtime | 未分摊 |
| 时间归属 | 可使用前一事件/turn 起点 | 当前 usage 事件时间；跨日 Claude 修订未分摊 |
| fork/replay | 含复杂 fork 状态机 | 现代 thread 身份校验；legacy 严格 UUIDv7 毫秒边界；含混归属拒绝或明确列 coverage |
| headless、工具内容 token 估算 | 支持多种附加路径 | 不估算；已知 usage 信封位于不支持的记录形态时拒绝，不递归搜索正文 |
| 模型 | 原始/规范化模型名 | 只输出私有配置 model_aliases 对应的受控标签，否则 unknown |

实际运行的是本仓 parser。未构建/运行完整上游 crate，不能宣称完成上游运行对拍或覆盖其全部语义。

## 构建依赖

精确版本由 `Cargo.lock` 固定。直接依赖：serde 1.0.229、serde_json 1.0.154、chrono 0.4.45、sha2 0.10.9、walkdir 2.5.0、tempfile 3.27.0、same-file 1.0.6。均为公开 Rust 基础库；serde/serde_json/tempfile/sha2/chrono 的已下载 Cargo 元数据为 MIT OR Apache-2.0，walkdir/same-file 为 Unlicense/MIT。本轮未对所有传递依赖进行安全审计；不把固定版本解释为安全保证。

生产依赖树没有 HTTP client、SQLite、GUI、provider SDK 或自动更新模块。使用 `cargo build --locked`，不调用 agent 程序、凭据加载或在线额度接口。

macOS arm64 已原生构建及运行；macOS x86_64 和 Windows x86_64 尚未验证。非 Unix 物理文件检测暂保留句柄且限制 4096 个文件，超限显式失败；Windows 扩展验证前不能称为生产支持。


p2a-2 进一步明确偏离：无累计基线时 last-only→total-only 拒绝；Claude fallback 身份与带 requestId 身份混用拒绝，而不是默认为两个调用。Codex 显式 usage 模型更新上下文已与参考规则一致。详见协议与代码审查修复记录。


p2a-3 另审查官方 openai/codex 固定 commit `092d3acd6bec3e3a14bdc7e7a2810ab628ab759d` 的 protocol/session/compaction 调用链，许可证 Apache-2.0。没有复制其实现或新增上游 crate 依赖；根据已确认数据契约新增本地 response 观测 adapter，证据见 [专项记录](codex-response-usage.md)。这不改变“未运行完整上游 tokscale-core parser”的验收边界。


P2b 引入固定 rusqlite 0.40.2（MIT），bundled libsqlite3-sys 0.38.2（许可证与 SQLite public-domain 来源见发布包元数据）。只启用 bundled，不引入扩展加载或 provider 网络模块。链接版本在运行输出中记录；readonly/WAL 行为由真实 adapter 的合成 DB 测试验证。当前该绑定的 bundled 运行库实测为 SQLite 3.53.2。此前“无 SQLite 依赖”的描述仅适用于 P2a 历史基线。agy 字段适配来源仍是同一固定 tokscale-core revision，保留现有 MIT 归属。

## Zstd event decoding

The OpenClaw adapter uses crates.io `zstd = 0.13.3` (MIT), pinned in Cargo.toml and Cargo.lock. Published crate provenance is zstd-rs revision `52e9d41a645fb49edcf1ed3df765f380dbcf13b2`. Locked transitive bindings are zstd-safe 7.3.0 and zstd-sys 2.1.1+zstd.1.5.7 (BSD-3-Clause), the latter from revision `9167d35a0d9bfdd7ce2c983c0cdc56acbdab6526`; bundled upstream Zstandard is 1.5.7. Licenses are retained under third_party/zstd, third_party/zstd-safe, and third_party/zstd-sys. Default Rust features are disabled; there is no dynamic extension loading, dictionary builder or provider initialization. Decoding enforces an explicit 16 MiB event cap. No private probe code was copied into the adapter.
