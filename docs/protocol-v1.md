# P2a 配置与快照 v1

权威可执行结构在 `src/protocol.rs`，解析器版本为 `p2b-4`。配置使用 JSON，拒绝未知键。CLI：

```sh
cargo build --locked
agent-usage collect --config PRIVATE_CONFIG --cache PRIVATE_CACHE
```

`--cache` 可省略。配置/缓存目录由调用者建立。程序不会扫描默认 HOME，不展开 `~` 或环境变量。配置和缓存路径可相对当前目录；`--cache cache.json` 将使用当前目录。source.paths 必须是绝对目录路径。只递归读取其中 `.jsonl` 文件；不自动纳入归档或相邻目录。普通非 JSONL 文件不读取；目录内 symlink 拒绝，根目录别名 canonicalize 后检查重叠。

## 配置

```json
{
  "schema_version": 1,
  "host_id": "example-host",
  "sources": [
    {"id": "codex-main", "adapter": "codex-jsonl", "paths": ["REPLACE_WITH_PRIVATE_ABSOLUTE_DIRECTORY"]},
    {"id": "claude-main", "adapter": "claude-jsonl", "paths": ["REPLACE_WITH_PRIVATE_ABSOLUTE_DIRECTORY"]}
  ],
  "model_aliases": {"example-provider-model": "example-model"}
}
```

这只是占位示意，需在仓外私有配置中替换目录。adapter 当前只支持 `codex-jsonl` / `claude-jsonl`；其他值返回 `unsupported_adapter`。source ID 不得重复。host_id/source_id/模型输出标签限 1–64 位 ASCII 字母数字及 `._-`，操作者应使用不含个人信息的受控标签。model_aliases 的键不会导出；未列入映射的原始模型名归入 unknown，保留 token 并标记 coverage。配置应保留于来源机器，不能入库。

指纹是结构化配置的 SHA-256，包含来源路径、顺序和 model_aliases；只导出摘要，不导出路径。来源/映射变化使缓存失效。

## 成功快照

- `schema_version=1`、`collector_version`、`parser_version`、`host_id`、`configuration_fingerprint`。
- `generated_at` 为 RFC3339 UTC，`timezone=Asia/Singapore`。
- `complete=true` 表示所有已配置来源读取成功；`coverage_complete` 表示当前解析范围内无已知缺口，不保证日志保留完整或历史从未删除。
- `sources[]`：source_id、status (`ok`/`empty`/`partial`/`unmetered`)、files、cache_hits、bytes_read、coverage。
- `rows[]`：source_id、agent (`codex`/`claude`)、model、day (`YYYY-MM-DD` 或 null)、time_basis (`event_timestamp`/`unallocated`)、tokens。
- `tokens` 和 `totals` 均为五个互斥非负 u64 整数：input、cache_read、cache_write、output、reasoning。求和溢出失败，不饱和截断。JavaScript 消费者须使用可保留 u64 精度的 JSON 解析方式。
- `cost=null`，未估算费用。

coverage 包含 usage_records、pending_files、unknown_total_carry、missing_time_records、unknown_model_records、unsupported_fields、counter_resets、streaming_regressions、unallocated_tokens。它们是记录/字段或 token 的计数，不含原始值。unknown_total_carry 是各日志累计 total 与已知 input+output 差额的最高值之和，不是可信可计费分项。相同日志的复制可能使此诊断金额重复；不可与 totals 相加。

缺时间或流式消息跨日时 day=null；不以文件 mtime 补时间。日桶加未分摊严格等于 totals。total-only carry 不在此等式内。unknown 字段只标 coverage，未证明语义前不强行解释。unsupported_fields 可能包括供应商新增的非 token 元数据，不能直接解释为缺失 token 数。

## 失败与原子性

失败 exit code 1，stdout 只有 `{"schema_version":1,"complete":false,"error":"固定错误码"}`，不带部分 rows/totals；stderr 只有固定诊断。输出 IO 失败 exit code 2。配置错误、缺失来源、读取失败、损坏完整 JSONL、unsupported 格式、非法 token、含混回退等都是失败；空目录明确 `empty`，有零计数记录则为 `ok`，不能互相替代。

失败不替换缓存。stdout 写出不保证传输原子性，P3 必须完整解析信封和验证 complete/配置指纹后才替换 last-good。P2a 不提供 last-good 存储。

## 缓存与一致性边界

可选缓存使用临时文件 + 原子替换，Unix 权限由 tempfile 限制为当前用户。内容仅为计数、时间桶、受控模型及哈希身份/路径，不能共享到远端或入库。缓存属于私有、可信的本地状态，未设计对恶意篡改的认证。损坏缓存报错，调用者可移走后重建。

未变文件按 size/mtime/文件身份（Unix 含 ctime）命中，不重读日志。变更文件重新解析整个文件；不是字节级增量 tail。末行没有换行时整个末行延期，后续追加后重新读取。读取前后元数据变化会报 `source_changed_during_scan`，调用者重试；跨文件不是文件系统全局事务。单次并发收集应由调用者避免；P3 再实现每机单飞。

缓存不会保留已删除文件的历史；成功重扫可能降低 totals。当前元数据失效策略针对普通本地文件系统，不承诺对保留所有元数据的外部内容篡改或网络文件系统强一致。活跃大文件的增量读取、所有历史 fork 兼容及原生 Windows 验证是后续准入项。


## 当前 CLI 与 coverage 补充

CLI 增加 collect --output PRIVATE_SNAPSHOT --summary、report --snapshot PRIVATE_SNAPSHOT，以及 JSON help。成功才原子发布快照；输出路径不能覆盖配置/缓存/来源，不能跟随输出 symlink。report 不采集来源，只验证并汇总保存的快照，仍需看 generated_at 判断时点。summary 使用 known_tokens / known_total_tokens，明确不是完整账单。

Codex raw counters 现在是 input-inclusive/cache-read/output-inclusive/reasoning/cache-write 五维，cache-write 是 input 子集，非缓存 input 同时扣除读写缓存。旧格式 fork 仅在 turn UUIDv7 毫秒部分严格晚于 child 创建部分时进入自己的用量；同毫秒、旧 ID、回退均不猜测。无法可靠归属的前缀仍列 coverage。

已识别无 usage 的早期 Codex transcript 增加 unmetered_files；不能估算成 token 或解释成真实零。Claude TTL 子计数超过父 creation 时增加 invalid_cache_ttl_records，并隔离消息的 cache-write，其他独立分项保留；不确定性跨流式版本与缓存传播。任一上述计数非零，coverage_complete=false；来源状态为 partial 或 unmetered。其他 IO、类型、JSON、身份与含混累计错误仍使整轮失败。

缓存 JSON 写入增加缓冲；文件集合/内容未变时不重写缓存。缓存与快照不是一个跨文件事务，单次失败可能留下可用新缓存但不会替换旧成功快照。CLI 已对共享配置/状态目录提供本地协作锁；P3 的远端每机单飞仍未实现。

详细操作与验证边界见 [最小可用版本](minimal-usage.md)。


## 本地协作锁

collect 自动持有配置、缓存和输出目录的 .agent-usage.lock，直到快照发布完成。使用非阻塞系统锁：冲突返回 collection_busy、complete=false、exit 1，不修改缓存/快照；report 继续只读。不同目录互不影响，同目录中不同文件也会互斥。锁标记为空且持续保留，进程退出由系统释放锁，不使用超时/PID 猜测回收。

获锁后与发布前的配置语义复核失败返回 configuration_changed。它不构成针对非协作编辑器的原子版本栅栏；修改配置应避开采集过程。库调用者需显式持有 locking::CollectionLocks 并覆盖 publish 调用。纯解析器与快照 schema/parser 版本不变。


## agy SQLite 来源

adapter=agy-sqlite 支持显式数据库文件或目录下直接的 .db/.sqlite 文件，不递归扫描备份。SQLite 不复用主文件 metadata 缓存，每次读取只读事务内的数据与已提交 WAL。信封/summary 的 sqlite_version 记录实际链接版本。JSONL 来源仍采用原缓存行为。

新增 coverage：missing_identity_records、ambiguous_time_records、unclassified_usage_records。前两项分别表示没有稳定 response 身份和时间关联冲突；最后一项表示不能把该行认作已完成调用。它们使 coverage_complete=false；unclassified 记录不加入已知总量。来源 agy 的数字映射和边界见 [adapter 文档](agy-adapter.md)。

## Hermes interval rows (parser p2b-2)

Rows optionally include `interval: {start, end}` (RFC3339 or null bounds), `usage_kind` (`main`, `auxiliary`, `residual`) and `backend` (reserved controlled label). Same-day aggregates use `aggregate_interval`; cross-day/open intervals use `unallocated_interval` with null day. Missing bounds are never inferred from filesystem metadata. Reports validate bounds and their day assignment. Added coverage counters are `auxiliary_usage_records`, `residual_usage_records`, `negative_residual_records`, and `orphan_usage_records`. Details: [Hermes adapter](hermes-adapter.md).

## OpenClaw rows (parser p2b-3)

`openclaw-codex-jsonl` assigns dedicated Codex usage to agent `openclaw`, backend `codex`. `openclaw-sqlite` adds qualified non-Codex message usage, retaining transport API as `backend` and an opaque provider label in optional `provider`. Both fields participate in grouping and replay checks. Coverage adds `runtime_usage_records` (diagnostic only) and `missing_backend_threads`. See [OpenClaw attribution](openclaw-adapter.md).

Parser p2b-4 caches retain Claude provisional/terminal usage stage for reconciliation across files and warm scans. Old caches are invalidated. Coverage adds `input_reclassifications` and `deduplicated_symlink_files`; unresolved provisional records contribute to `unclassified_usage_records` and not known totals. See [the accounting refinements](data-contract.md#observed-streaming-refinements-p2b-4).

Failure envelopes may include a controlled `source_id`; null means the error is not attributable to a source. Fleet hosts optionally include `failed_source_id` alongside `error`. The app-owned SQLite store uses schema 2 for this metadata; source DB schemas and JSON envelope version are unchanged. See [operation and rollback semantics](operations.md).

`config-id --config PRIVATE_CONFIG` returns a schema-1 `configuration_identity`
with host ID and the same canonical configuration fingerprint used by collect.
It reads only the typed configuration, without scanning or creating sources.
This is an identity operation, not source-availability validation.

## Additive metered-count field

Parser p2b-5 adds `rows[].messages` with `known` (u64) and `complete` (bool).
The field counts deduplicated metered assistant responses/API calls; it is not
all user/tool messages or conversation turns. Missing legacy fields decode as
unknown, never as verified zero. Aggregation checks u64 overflow and propagates
partial coverage. Token buckets and snapshot schema version remain unchanged.
`report` adds the aggregate `messages` object. Versioned monetary estimates are
computed by the UI's offline catalog and do not overwrite snapshot `cost`.

## Additive analysis dimensions (parser p2b-8)

Rows may carry `hour` (integer 0–23 in Asia/Singapore) and `execution_role`
(`main` or `subagent`). Both are optional; absence means unknown. Hour requires
a valid `day` and `time_basis=event_timestamp`. Aggregate intervals are never
assigned an hour. Hour and role participate in grouping, so consumers must
include them in row identity and reaggregate rows for daily/model totals.
Old daily-only snapshots remain valid and explicitly lack hourly coverage.
These dimensions do not change token totals or metered response count semantics.
