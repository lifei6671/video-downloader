# 备份网络稳定性与操作列表性能验证

日期：2026-11-05。基线：`24cb3064de18e7f92509ec7963420edcf84f2646`。范围为本轮审查文档的六项优先问题；未提交或推送。

## 实现

1. S3 文件 PUT 和文件分片请求使用 `60s + bytes / 64KiB/s` 向上取整的独立 read/operation 预算，连接上限仍为 15 秒。启用 SDK 上传停滞保护，宽限期 20 秒；元数据仍使用原短超时。取消沿原任务 future 回收路径生效。
2. Telegram 在准备附件、读取目标和生成 rich message 成功后，紧接实际发布 RPC 前同步持久化 `Sending`。准备阶段失败仍是 `Prepared`；已发出但回复丢失仍只对账，不盲目重发。
3. 已初始化 sender 中未取消的文件 RPC 超过 24 秒、控制 RPC 超过 55 秒未完成时返回超时，按连接 ID 淘汰。媒体连接进入现有降级和冷却；恢复探测失败关闭被探测 generation，保留授权及发送日志。取消请求不参与健康超时判断。
4. v8 增加轻量操作投影和同事务触发器，界面按空间通过独立只读 WAL 连接读取，不持有主任务锁。上传进度每秒只更新变化的独立列，避免反复解析、序列化和重写大 payload；恢复检查点保持原业务边界。按需恢复改为按空间、资产及状态直接查询。
5. S3 分片对网络错误、503 等服务错误、限流最多尝试 3 次，退避 1/2 秒并每 100ms 检查取消；始终重用 upload ID、分片编号和字节范围。Create/Complete 与条件写不进入此局部重试。跨任务重启仍沿既有清理与重传契约。
6. Telegram 已知目标消息的常规附件更新直接读取该消息，保留归属、operation/kind 与内容检查；缺少可靠位置或发送结果未知时仍使用历史查找和重复操作检查。

SDK 的 read timeout 从请求开始计时，不能当作上传完成后的纯响应等待上限；参考 [AWS Rust 超时说明](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/timeouts.html)。同 upload ID/part number 重传替换该分片，参考 [S3 multipart 说明](https://docs.aws.amazon.com/AmazonS3/latest/userguide/mpuoverview.html)。

原恢复 payload、封面及文案快照保留，未迁移为新的共享快照存储。已有 UI 用历史操作计算各资产状态，因此保留当前空间的完整薄历史；未新增历史分页页面或改变保留策略。文档列为后续处理的重复摘要计算、跨 DC 授权合并、多空间 N+1、S3 客户端复用不在本轮范围。

## 本地性能测量

Windows debug Rust，真实临时 SQLite/WAL，每条操作包含约 128 KiB 的 payload；旧路径为持有主锁的七字段 `json_extract`，新路径包含读取及构造完整 UI Operation。单次测量，用于验证本轮瓶颈，不代表线上 P95。

| 历史记录数 | 原查询 | 新投影查询 |
| --- | ---: | ---: |
| 500 | 141.821 ms | 3.792 ms |
| 2,000 | 434.738 ms | 7.620 ms |
| 5,000 | 1,149.092 ms | 19.858 ms |

独立读连接锁保持占用时，取消操作在 500ms 测试期限内完成。重复相同进度不会产生额外数据库变更，原恢复 payload 内容不变。

## 实际验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib --locked -- --test-threads=4` | PASS，490 passed / 27 ignored |
| `cargo test --manifest-path src-tauri/Cargo.toml --test backup_migration --locked` | PASS，6 passed / 1 ignored |
| `cargo test --manifest-path vendor/grammers-mtsender/Cargo.toml --lib --features proxy --locked` | PASS，20 passed |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets --locked -- -D warnings` | PASS |
| `cargo clippy --manifest-path vendor/grammers-mtsender/Cargo.toml --lib --features proxy --locked -- -D warnings` | PASS |
| `npm run build` | PASS，TypeScript 与生产前端构建 |
| `git diff --check` | PASS |

回归包含：本地 TCP 持续上传超过 30 秒后成功；停滞上传在 45 秒测试期限内失败；同会话分片瞬态恢复、连续失败恰好三次、退避取消及清理证据；Telegram 准备失败零发布、重试一次发布、回复丢失恢复；已初始化静默控制 socket 重建、静默媒体 socket 回退到健康控制连接、取消后复用健康连接；v7 回填、v1/v5/v6 升级、失败回滚和完整备份。

连接测试使用合成授权 key 和回环 TCP。控制 socket 重建测试发出的是文件 RPC，覆盖 24 秒分支；普通控制 RPC 的 55 秒分支没有单独新增回归。已知附件零历史读取按调用路径核查，未用真实频道计数验收。真实 Telegram/S3 网络、原生 Tauri 运行、macOS、安装包验收：`NOT_RUN`，未使用真实用户凭据。忽略项不计通过。前端保留既有 sourcemap / bundle size 提示。

独立交付复核采用 S3、Telegram、数据库/界面三个分区，加最终集成审查；最终结论以交付时冻结目标为准。
