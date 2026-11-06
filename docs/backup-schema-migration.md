# 备份 SQLite 兼容迁移

2026-11-04。用户授权为正常应用数据目录增加兼容迁移；保留本轮已有 Telegram 分片和解析引擎修改，不提交、推送或恢复线上传输测试。

## 原因与支持范围

正常应用目录的 `backup_schema.version=5`，当前重构实现曾只接受 1。旧空间表有 10 列、远端状态表有 9 列，现有实现分别使用 7、4 列。修改版本数字不能转换这些结构。

新建库使用 **v9**，支持以下本地升级：

- 当前规范布局的 v1 → v9：保留现有同步设置及任务恢复语义。
- 已核验三表列名/顺序、提供商协议 2 的 v5 → v9：转换本地表结构，保留数据并暂停自动同步。
- v6 → v7：保留同步设置，新增按空间记录的删除隐藏状态，修复另一空间仍有效的资产被全局隐藏。
- v7 → v8：增加独立的备份操作显示投影，回填已有状态，保留原恢复 payload 和同步设置。
- v8 → v9：增加资产反向索引、独立上传捕获表和导入回执。未完成、取消和待确认操作的完整捕获保留；已完成操作释放封面和文案正文，身份与发送证据保留。
- v9 重复启动幂等，不重复备份或转换。

这里不承诺任意手工修改过的 SQLite 结构兼容。未知版本、三表布局不符、非协议 2、未决 `migration_json`、非空 `backup_probe_cleanup` 均明确失败。重建的提供商配置表若有额外索引、触发器或任何入向外键，也拒绝迁移；标识符比较遵循 SQLite 不区分大小写的规则。不更改远端协议、库对象或 Telegram 消息格式。

## 删除隐藏状态

`asset_hidden_history` 继续保存用户主动移除列表或任务历史的全局标记。删除成功产生的隐藏状态改存 `asset_deletion_hidden_history(space_id,asset_id)`，只影响本次删除的空间；其它有效成员仍可显示和恢复。重新上传到原空间时清除此空间的删除隐藏标记。

升级时，仅将具有 `local_cleanup_done=true` 的持久删除操作对应的旧全局标记移至空间表；不把文件缺失或单纯的远端墓碑视为本机删除证据。无该证据的主动隐藏保持原样。表创建、旧标记迁移和版本变更在既有事务中完成，失败全部回滚。v6 升级不会停用同步，也不触发任何远端写入。

## v8 操作状态投影

`backup_operation_display` 保存界面需要的状态及轻量摘要，通过同事务触发器随操作插入、状态变化、payload 检查点和删除同步。v9 起封面、文案等不可变上传捕获存入 `backup_operation_captures`，与首次检查点同事务保存；进度、重试和发送证据继续保留在操作记录中。上传字节进度写入投影的独立列，每秒仅写变化值；原恢复检查点继续按分片等业务边界持久化。投影不作为恢复发送或认定上传成功的依据。

界面按空间读取投影，使用独立只读 WAL 连接，不持有主任务锁。已有界面依赖历史操作判断各资产状态，因此保留薄历史记录；没有新增历史列表页面或更改历史保留策略。按需恢复直接查目标空间和资产的未完成恢复记录。

## v9 查询与恢复数据

`backup_space_assets(local_asset_id)`、`backup_operations(asset_id)` 支持历史补登记反向查询。历史登记按任务状态、路径、资产关联变化及成员移除失效，启动时重新检查；缺失本机文件的候选每 30 秒复查。单项资产操作按主键查询，不加载整个资产列表。

`backup_import_receipts` 同时比较远端记录、远端版本、本地版本和待上传状态，命中时不重复导入。共享本机资产被任一空间重写会失效所有相关空间的导入回执，避免空间切换后保留错误投影。回执不作为备份成功、删除归属或数据完整性的证据。

上传发布确认后，在本地确认事务中删除独立捕获正文；旧操作完成取消清理并被显式重传或重建请求替代时，也在终结事务中回收正文。启动会回收旧版本遗漏的已完成操作捕获。取消、失败、未知发送结果和仍可重试的任务继续保留完整捕获及清理依据，操作身份和发送回执不随捕获正文删除。v8 升级中索引、捕获搬移和版本更新属于同一个事务，失败全部回滚。

## 执行路径与备份

`Core::open` 取得现有实例锁后，在任何启动 schema/cache 写入前调用 `backup::schema_migration::prepare`。识别到升级时立即通过 `repository::init_prepared` 迁移；普通新库/当前库仍沿原初始化顺序。

已存在的文件库在原目录生成：

```text
yingqu.sqlite.before-backup-v5-<UUID>.sqlite
```

使用现有 rusqlite 的 SQLite 原生 online backup API，包含已提交 WAL 页且保留 rowid。先写私有临时文件、同步落盘，再以不覆盖方式保存；备份失败不会开始迁移。内存 fixture 没有持久用户库，不生成文件。依据：[SQLite online backup](https://www.sqlite.org/backup.html)。

迁移 DDL、同步暂停、版本变更、外键检查及原恢复操作处于同一事务。执行失败整体回滚，完整旧库备份保留。不会关闭外键检查，不使用 `writable_schema`，不加载账号会话或执行网络请求。

## v5 数据保留与任务语义

以下五张表按原列、原值保存在本库的 `backup_legacy_v5_*` 归档中：

| 原表 | 归档后缀 | 当前表处理 |
| --- | --- | --- |
| `backup_spaces` | `spaces` | 移除 role/writer 字段，保留目标、库身份和凭据引用；enabled 设为 0 |
| `asset_remote_objects` | `remote_objects` | 移除旧对象键/etag，保留空间、资产、修订和状态 |
| `backup_provider_config` | `provider_config` | 重建规范列与 protocol=2 约束；不提升旧远端协议 |
| `backup_operations` | `operations` | 原表保留，只有 pending/running/retry_wait 改为 blocked，标明需核对 |
| `backup_sync_preference` | `sync_preference` | 当前 enabled 设为 0 |

已有 blocked/completed/canceled 操作的状态、phase、payload 和错误不改写。`telegram_media_sends`、random_id、操作 ID、文档引用和原回执原样保留。资料库、媒体、文案、封面、设备身份及其它原有表不重建，不补录历史下载。

暂停同时写入空间 enabled 和全局偏好，因为发现及维护不都依赖全局开关。未决探针清理会绕过这些开关，因此发现该记录时先拒绝迁移，不能在启动后替旧版本自动 Abort/Delete。没有把完成的删除重新入队。

迁移后可查看本机原资料。确认旧远端库与当前格式兼容后，再从已有设置显式重连/开启同步；未决任务仍需核对原证据，不自动重发。恢复备份应先完全退出应用，使用完整 SQLite 备份及兼容版本，避免将备份覆盖到仍打开或有未处理 WAL 的数据库上。

本机后续经显式授权完成的旧云端读取兼容、三条 R2 任务对账、历史 Telegram 证据封存及同步恢复见 [旧云库兼容与恢复](backup-cloud-compatibility-recovery.md)。它们不属于启动时自动迁移，不改变本节的暂停与回执保留约束。

## 已执行的专项校验

未恢复全库回归、Telegram/S3 线上测试或性能矩阵。仅针对这次迁移执行：

- `cargo test --locked --manifest-path src-tauri/Cargo.toml --test backup_migration`：5 passed、1 默认忽略。覆盖 WAL 备份、资料及发送证据保留、暂停、重启幂等、注入失败回滚、v1、未知版本/布局/协议、未决升级/清理、额外索引及大小写入向外键拒绝。
- `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --lib --test backup_migration -- -D warnings`：通过。
- 显式启用 `migrate_authorized_database_copy`：1 passed。路径严格限制到本轮私有副本，不读取真实会话或联网。
- 副本逐表核对：59 张原有表逻辑内容完全一致，五份归档完整；其余投影字段一致，仅按约定转换结构、版本和同步开关。迁移前自动备份的原表内容也全部一致；完整性为 ok，外键错误为 0。

副本保留 143 条资料、169 条备份资产、2 个空间、175 条操作及 25 条 Telegram 发送证据；原有 3 条 blocked 操作内容未变化。证据在忽略目录 `.local/backup-schema-migration/`，不将原始库、账号、凭据或业务内容加入仓库。

正常标识 Release 随后构建完成（`--locked`，1m23s），使用本轮 `app/yingqu.exe` 启动原 `me.disign.yingqu` 数据目录，窗口“映取”保持响应，启动 stderr 无错误。原库已由 v5 迁至 v6，完整旧库快照保存在原库同目录。

首次启动后再次只读核对实际原库与其自动备份：59 张非转换表内容完全一致，五份归档及投影字段一致，完整性 ok、外键错误 0；143 条资料、169 条备份资产、175 条操作及 25 条发送证据保留，状态仍为 172 completed / 3 blocked。所有空间和全局自动同步均关闭。结果为 `original-preservation.redacted.json`，启动记录为 `launch.json`，构建记录为 `build-result.json`。没有据此宣称旧远端格式已兼容或线上传输已验收。
