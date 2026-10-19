# 开发计划

当前为 experimental 单机最小版本，支持 Codex/Claude/WorkBuddy JSONL、agy/Hermes/OpenClaw SQLite 及专属 OpenClaw Codex 来源。参见 [使用说明](minimal-usage.md) 与 [验证状态](minimal-validation.md)。已实现按需多机汇总；历史 coverage 缺口和完整试运行验收仍待完成。

## 仓库与依赖

本仓目前维护 Rust collector、配置、版本化 JSON 协议、本地快照和测试。已实现按需本地/SSH 调用与 SQLite last-good 汇总；原生界面位于独立 Syrtis fork，多机 UI、已计量调用数和目录价估算已接入。

优先评估固定 tokscale-core revision `fcb85923e5544488f98001cfa390740d0009b9da` 的本地 parser。该版本来自 Syrtis v2.7.0 调查基线，不代表可以原样满足全部来源。已完成许可证、依赖和 parser 接口审查。本轮采用窄范围规则适配，未引入整个引擎；原因与行为差异见 [依赖评估](dependency-review.md)。collector 已有 macOS arm64/x86_64 和 Windows x86_64 GNU 原生验证；UI 复用独立 Syrtis fork 的原版 target。

## 分阶段交付

1. **P2a：本地最小闭环。** 初始化 Rust crate，明确配置与快照 schema，实现多目录 Codex/Claude adapter、累计差分和流式更新处理。测试实际 parser，随后在开发机器只读对拍统计字段。未知 total carry、缺时间与异常字段要进入 coverage。
2. **P2b：补足来源。** agy SQLite/protobuf、Hermes 主/辅助调用和跨日未分摊、OpenClaw SQLite/Zstd 与 Codex 后端归属。补同调用多存储的人工 fixture；不以模型名决定后端。逐平台验证 macOS arm64/x86_64、Windows x86_64。
3. **P3：汇总。** 本地 CLI 已实现共享配置/状态目录的协作锁与发布前配置复核；后续仍需 SSH 超时、远端每机单飞、整机快照事务替换、配置版本匹配、last-good 恢复与跨机简单相加。默认建议五分钟轮询、远端并发二；实际参数留在私有配置。
4. **P4：界面。** fork Syrtis，使用独立 bundle ID、存储目录和手动更新路径；展示全部/单机、模型、日期、来源与陈旧状态。后台采集不阻塞 GUI。
5. **P5：试运行。** 四机真实运行 24–48 小时，验证离线恢复、重复刷新、活跃 WAL、重启和资源开销，记录回滚步骤。

## 验收要求

- 各 adapter 的实际实现通过人工 fixture；上游 parser 与参考预期的差异需解释。
- 同一快照重复导入十次不增加总量；两个 host 的同样记录恰好计两份。
- 已启用来源任一失败则本轮整机不发布；无 last-good 时显示未采集，不显示零。
- 不完整 JSONL 末行延期处理，完整后正确计入；计数回退必须有显式策略。
- SQLite 只读一致性读取包含已提交 WAL，记录实际链接版本；错误不能变成空结果。
- 日桶加未分摊等于总量；未分类来源单列，不作为可信完整总量。
- 重叠/重复物理路径拒绝或去除；不自动扫描归档、备份、虚拟机。
- 暖扫描避免反复全量读取大日志，冷/暖性能以实测记录。
- 凭据读取和 provider 请求路径不存在；导出及诊断只含允许字段。

费用首版可省略或标 unknown；若加入，只是带价格版本的估算。永久历史账本、跨机去重、在线额度、远端常驻服务均延期。原始日志删除后成功重扫可能降低历史总量，必须说明。

WorkBuddy 首版范围与覆盖限制见 [adapter 说明](workbuddy-adapter.md)。
