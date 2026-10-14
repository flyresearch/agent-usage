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

支持 XDG_CONFIG_HOME/XDG_DATA_HOME，也可通过 AGENT_USAGE_CONFIG、AGENT_USAGE_STATE_DIR 指定位置。目录须已存在。

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

## 使用

    ./scripts/usage.sh refresh
    ./scripts/usage.sh report

refresh 读取配置并采集，成功后原子替换快照；report 仅汇总已保存快照，不扫描来源。刷新失败后 report 仍可能显示旧结果，请检查 generated_at 和刷新退出码。不要同时运行多个 refresh。

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
