# 旧云库兼容与恢复

2026-11-04，Windows 原数据目录。本轮不运行性能测试，不提交、推送或安装应用。

## 后台恢复登录

`desktop.rs` 仅启动一个异步任务调用 `TelegramAccounts::restore_saved`，不在 `Core::open`、窗口创建或启动返回路径中等待网络。共享会话按 Arc 去重，独立账号分别恢复，不自动切换绑定或启动扫码。

会话初始状态为 `disconnected`，恢复时为 `connecting`；网络、超时及凭据存储错误保留重试入口。只有服务端确认未授权、明确认证失效或用户完成退出后才为 `signed_out`，界面此时提供扫码。登录恢复不等于开启云同步；代理、取消和应用退出沿用原资源回收流程。

## 已核实的旧格式

R2 描述仍为协议 2，但缺少后来新增的 `deletion_indexes`，默认 false。162 条记录中 161 条使用历史深层键，包括内容摘要目录和 device/operation 目录；157 条历史记录的 writer 为空，仅保留未知身份。新上传仍使用浅层内容键，未知路径、跨资产路径、摘要和扩展名不符仍拒绝。旧深层对象的显式删除及自动清理返回 `BACKUP_LEGACY_READ_ONLY`，不会 Delete 或 Abort。

Telegram 同频道有 48 条现存消息，包含旧独立消息、旧富消息与当前富消息。读取支持旧单锚/分块 manifest，保留无序号事件的原始消息 ID；当前事件继续使用 `(message_id << 32) | sequence`。删除代际校验同时理解两种修订。旧 v1 可替换事件中的完整 base 仅用于恢复其已验证快照，不能替代缺失的原始游标证据。

一个历史资产的引用未保存 document ID；只能从真实同频道、同消息、同资产、同摘要/字节数/容器的文档补齐，不给当前 MediaRef 增加缺省 ID。25 条历史发送日志仅兼容已知 `use_document_identity=true` 标记，保留原 nonce、操作 ID、回执及发送状态；false 和其它未知字段仍拒绝。

本地消息映射增加 reader_format=2 作为缓存键，重建旧解析缓存。已观察游标不清空、不跳过不一致检查。

## 历史证据恢复

旧游标中有两条事件已不在云端现存消息/归档里。它们均有 Confirmed 发送日志和 completed 操作 payload；按原 Event 编码重构后的原始 SHA-256、长度及消息身份完全匹配。补齐它们后保留全部 16 条原始游标摘要，重放得到 13 个活动资产及 333 个墓碑。9 个本地已缓存资产的原视频摘要、字节数与容器均保持一致。

修复计划采用追加当前格式库索引，previous_segment 链接原有归档头，仅保存两条原始事件字节。不覆盖旧消息、不重传原视频、不删除旧载体。执行前重新验证归档头、原回执和源消息确已缺失；新操作 ID 固定在受保护计划中，通过原 Prepared → Sending → Confirmed 和 ReadBack 路径发布。发布未知时不能生成新 ID 盲目重试。

## 三条 R2 受阻任务

三条均为旧 upload，错误 `BACKUP_LEGACY_OUTCOME_UNKNOWN`。逐条核对记录键、media/transcripts 目录和原 payload 指定对象均不存在，源文件仍在。实际执行前再次检查全部三条，确认没有 baseline、冲突、删除或重新创建状态，源 SHA-256 与只读审计一致，再保留原操作 ID 接入现有 SyncEngine。

已完成三条，独立重新下载共 65,076,528 字节，SHA-256 全部相同。仅新增 3 个视频、3 个封面和 3 条记录；文件 PUT 无重复键，Delete/Abort 为 0。恢复用 ObjectStore 包装层仅允许这三个资产的当前文件键与记录键，禁止 Delete、DeleteIfMatch、Abort；结束时恢复原空间选择及开关，期间不启动同步服务。

## 验证与运维入口

`compatibility_online_audit.rs` 和 `compatibility_online_recovery.rs` 是默认 ignored 的显式运维入口。默认离线测试不读取原数据目录或真实凭据。真实操作要求独立的 audit/recovery opt-in、同目录既有一致性快照、已退出旧应用；不复用合成媒体或性能矩阵入口。

受保护的本机证据位于 `.local/backup-compatibility-recovery/`，原始库、凭据、账号和业务内容不进入仓库。原备份及迁移归档继续保留。

已执行的正确性验证：

- `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1`：411 passed、0 failed、29 ignored。包含迁移、Telegram 会话/下载/上传/删除与旧事件、S3/R2 同步/恢复及相关集成测试；真实与性能入口默认忽略。
- `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`：通过。
- `npm run build`：通过。
- 原 current-only 测试的深层路径拒绝断言改为拒绝旧路径 SHA 错配，保留其余 13 项非法输入断言；另覆盖合法旧路径读取成功及删除/Abort 零写、未知 writer 范围、旧 nonce 保留、缺 ID 补齐及原游标字节、旧墓碑边界。
- 显式 R2 恢复：3/3 完成并独立 SHA-256 回读通过；旧 162 条记录 ETag 全部未变。
- 显式 Telegram 封存与再读：1 条新索引、0 视频上传、0 删除，原 16 条游标摘要全部保留；当前 700 条事件证据可完整读取，6 处本地缺失引用已补齐。
- 收尾复验：无未完成操作，integrity_check=ok、外键错误 0；已恢复原默认 Telegram 空间同步，R2 选择状态保持原状。

新版原生启动记录另在受保护目录保存；macOS、本轮性能测试为 NOT_RUN。
