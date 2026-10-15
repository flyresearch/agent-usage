# 本地配置与使用

当前为 experimental 单机 Codex/Claude CLI。每个使用者都需要建立自己的配置；仓库不包含来源日志、可直接使用的实机配置或预生成快照。

## 构建与配置

    cargo build --release --locked

在仓外建立配置目录和状态目录（Unix 建议目录权限 0700、配置文件权限 0600）。使用 convenience script 时，默认位置为：

| 文件 | 默认位置 |
|---|---|
| 配置 | $HOME/.config/agent-usage/config.json |
| 缓存 | $HOME/.local/share/agent-usage/cache.json |
| 快照 | $HOME/.local/share/agent-usage/snapshot.json |

支持 XDG_CONFIG_HOME/XDG_DATA_HOME，也可通过 AGENT_USAGE_CONFIG、AGENT_USAGE_STATE_DIR 指定位置。目录须已存在，且配置、缓存、快照所在目录需要允许创建锁标记文件。

配置示例中的路径是占位符，必须替换为自己明确授权读取的绝对目录，不会展开 ~ 或环境变量：

```json
{
  "schema_version": 1,
  "host_id": "example-host",
  "sources": [
    {"id": "codex-main", "adapter": "codex-jsonl", "paths": ["REPLACE_WITH_PRIVATE_CODEX_DIRECTORY"]},
    {"id": "claude-main", "adapter": "claude-jsonl", "paths": ["REPLACE_WITH_PRIVATE_CLAUDE_DIRECTORY"]}
  ],
  "model_aliases": {"example-provider-model": "example-model"}
}
```

仅启用需要的来源；可以只保留一个 source。路径可以多个，但禁止重复、重叠或重复物理文件。不要将完整 HOME、归档、备份或登录凭据目录当作来源。

model_aliases 的键是本地来源中的模型名，值是允许导出的标签。值限 1–64 位 ASCII 字母数字及 ._-，应选不含私人信息的标签。未配置的模型归入 unknown，仍保留 token。修改配置会使缓存失效。

这些值也直接用于 UI 模型名称。日常使用应为已核对的模型配置可读名称；不要把验证阶段的 `model-哈希` 匿名标签当作最终显示名称。标准模型名符合标签限制时可原样映射；含 `/`、`:` 等字符时显式选择可读别名。多机配置修改后须同步更新 fleet 中对应的配置指纹，再重新采集；不要直接篡改保存的快照。

## 使用

    ./scripts/usage.sh refresh
    ./scripts/usage.sh report

refresh 读取配置并采集，成功后原子替换快照；report 仅汇总已保存快照，不扫描来源。刷新失败后 report 仍可能显示旧结果，请检查 generated_at 和刷新退出码。共享配置或状态目录的 refresh 会立即返回 collection_busy，等当前采集结束后重试即可；report 无需等待。

也可直接指定所有文件：

    ./target/release/agent-usage --help
    ./target/release/agent-usage collect --config PRIVATE_CONFIG --cache PRIVATE_CACHE --output PRIVATE_SNAPSHOT --summary
    ./target/release/agent-usage report --snapshot PRIVATE_SNAPSHOT

省略 --summary 输出完整快照；省略 --output 不保存；省略 --cache 全量重读。stdout 始终为 JSON，错误诊断到 stderr，失败非零退出。输出不能覆盖配置、缓存或来源，也不跟随输出 symlink。Unix 新建的缓存/快照权限为 0600。

## 解释结果

complete=true 表示本轮来源读取/解析成功。coverage_complete=false 表示已知缺口。known_total_tokens / known_tokens 是按当前规则可恢复、可归属的分项，不是完整账单。

- unmetered_files：已识别早期格式没有 usage；不能当成从未使用。
- unclassified_fork_prefix_records：缺少可靠归属的 legacy fork 前缀未计入子线程可信用量。
- invalid_cache_ttl_records：Claude TTL 子计数超过 creation 父计数，该消息的 cache-write 被隔离为未知，其他独立分项保留。
- unknown_model_records、unsupported_fields 等保留模型/字段覆盖限制；字段个数不是未知 token 的数量。

负数、非整数、损坏 JSONL、缺失来源、身份冲突和含混累计切换仍失败，不发布部分快照。日桶加未分摊等于已知总量。费用当前为 null。

## 限制

仅 macOS arm64 已原生验证；其他平台不作生产承诺。变化文件整文件重读；未变文件免重读，未变缓存不重写。删除源日志会降低下一次成功快照的总量。缓存与快照各自原子替换，不是跨文件事务或永久账本。


## 并发与配置变更

CLI 在配置、缓存和快照目录中使用固定的空文件 .agent-usage.lock 持有操作系统排他锁，范围为目录：同目录中的不同配置或状态文件也会互斥。不同目录且不共享状态的采集可独立运行。该文件仅是稳定的锁载体，不保存 PID、路径或配置内容；Unix 新建权限为 0600。

进程正常退出或被终止后，操作系统自动释放锁；标记文件仍保留。采集期间不要删除标记文件或移动这些目录，否则可能破坏锁身份。锁是本地、协作式保护，不是远端分布式锁；旧版本 CLI 和直接调用 library 的程序不会自动参与。

CLI 获锁后和发布前会重新读取配置，语义变化、删除或损坏返回 configuration_changed，保留旧快照；仅空白/键顺序变化不会失败。这是边界复核，不是配置编辑器与快照之间的原子事务，仍应避免在采集过程中修改配置。发生晚期配置变化时，可重建的缓存可能已更新，但不会发布旧配置的快照。

配置/状态目录不能放在来源日志目录内；锁文件不能是 symlink、目录或含内容的现有文件。collection_busy、lock_open_failed 等错误不会泄露实际路径或秘密。

Hermes databases use `adapter: "hermes-sqlite"` and explicit database file paths.
See [Hermes accounting and interval coverage](hermes-adapter.md); directory roots
are not supported for this adapter. Keep this configuration outside the checkout.

OpenClaw uses paired explicit `openclaw-sqlite` database files and
`openclaw-codex-jsonl` dedicated session directories. Keep the dedicated path
out of ordinary Codex sources. [Attribution and limitations](openclaw-adapter.md).
