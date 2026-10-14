# 开发入口

先读 README.md、docs/development-plan.md、docs/data-contract.md、docs/handoff.md。

- 采用 KISS：单仓模块化实现 CLI、adapter、协议和汇总，不拆微服务。
- 实机配置、用户名、SSH 地址、绝对路径、原始会话/数据库、凭据不得入库。测试使用人工构造 fixture。
- SQLite 来源只读且必须看见已提交 WAL；禁止对实时库使用 immutable、改 journal mode 或主动 checkpoint。
- stdout 仅输出协议 JSON，诊断到 stderr；诊断不能带正文、工具参数或秘密。
- 必须区分失败、缺失、不支持和真实零用量；不得静默将失败转换成零。
- 不继承 GUI 的凭据、provider 网络调用或自动更新初始化路径。
- 复用第三方代码前核查许可证、来源及固定 revision；不要直接复制私有探针或未经审查的上游模块。
- 测试须调用实际实现；参考样例和算术 oracle 通过不代表 parser 验收通过。
- commit message 使用英文；提交、推送、发布和远程部署按用户明确任务执行。
