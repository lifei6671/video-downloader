# 备份审查缺陷修复验证

日期：2026-11-05。范围：本轮审查的五项功能缺陷，以及直接相关的小范围查询优化。未提交、推送或发布。

## 修复结果

1. 封面读取优先保留本地待同步选择；远端读取完成后，在事务中比较缓存内容和各空间的本地版本，避免旧请求覆盖已经采用并上传的封面。
2. 删除隐藏按空间保存；其他空间的有效成员继续显示并可恢复。显式隐藏历史仍保留全局语义。删除通知携带操作来源空间，切换空间后收到迟到通知也不会隐藏新空间的条目。
3. 自动重试与手动重试都在持久化新尝试时清除旧 `commit_intents`，避免启动恢复后的唯一键冲突。
4. Telegram 普通 DC 的连接和握手移到独立任务，同 DC 共享建连，其他 DC 不受阻塞；连接支持超时、取消与退出回收，并保留所有等待者的原始错误类别。
5. Telegram 媒体信息修复同时补齐资产缺失的宽、高和时长，并登记对应同步字段；保留用户已编辑的元数据。

附带优化：上传按操作 ID 查询；启动恢复先过滤候选再复制任务，避免触发全量完成文件检查；前端资产去重使用集合。操作 payload、全历史扫描等较大性能重构不在本次修复范围。

数据库版本升为 v7，支持 v1/v5/v6 升级，沿用升级前备份。仅把有删除完成操作证据的旧全局隐藏迁移为空间隐藏；无此证据的显式隐藏保持原状。迁移失败回滚与重复启动均有回归覆盖，见[迁移说明](backup-schema-migration.md)。

## 实际验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=4` | PASS，498 passed / 30 ignored |
| `cargo test --manifest-path vendor/grammers-mtsender/Cargo.toml --locked --lib --features proxy` | PASS，17 passed |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASS |
| `cargo clippy --locked --manifest-path vendor/grammers-mtsender/Cargo.toml --lib --features proxy -- -D warnings` | PASS |
| `npm run build` | PASS，包含 TypeScript 检查与生产构建 |
| `tests/backup-space-visibility-browser.mjs` | PASS，3 个视口 × 2 种布局 × 正常/迟到通知，共 12 场景 |
| `tests/backup-delete-flow-browser.mjs` | PASS，3 个视口 × 2 种布局 × 任务/纯资产，共 12 场景 |
| `git diff --check` | PASS |

浏览器使用生产构建和 synthetic IPC；连接池使用本地回环服务；备份与迁移使用离线测试数据。核心回归覆盖封面冲突选择、延迟写回、跨空间恢复、升级失败回滚、启动自动重试、媒体信息同步以及跨空间删除事件。

真实 Telegram/S3 在线验证、原生 Tauri 运行验收、macOS 和安装包验收：`NOT_RUN`。未使用真实用户凭据。忽略测试不计为通过。构建保留既有 sourcemap / bundle size 提示，Rust 测试包含 Windows linker 信息提示。

## 交付复核

基线：`e8c8e44b4458907bfffbd040f675f9f620ec3fe0`，`codex/s3-video-backup`，初始工作区干净。

审查目标：本轮 23 个实现、测试和相关文档文件；目标清单保存在本地 `.local/review-fixes-target.json`，SHA-256：`F04D51E455DFF34DA7D90B49BECDC56AE2F7C44B67C913BD52C2AE5496610EE5`。本验证记录为纯文字交付证据，不属于可执行目标。

审查方式：`CHILD_AGENT`；策略：`PARTITIONED_PLUS_INTEGRATION`。数据/迁移、连接池、UI/IPC 分区及最终集成均采用独立子 Agent 只读复核。

最终结果：`APPROVED`。Review gate：`PASSED`；Coverage：`COMPLETE`；Freshness：`FRESH`。修复轮次：2（上限 3）。独立集成审查重算全部目标哈希，0 处不匹配；无剩余 P0/P1/P2。

复核修复了封面缓存相同但版本变化的延迟写回、显式隐藏语义、连接失败错误分发，以及迟到删除通知的空间归属。未改变无关功能或增加依赖。
