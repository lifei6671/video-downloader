# 多设备同步实施与验证记录

实施基线：`codex/s3-video-backup` / `ed24459456e545833db68791cc53daa8fd52599c`。

最终状态（2026-10-31）：P1–P5 实现及适用离线/浏览器验收通过；P6 全量离线、真实 R2 v2 双设备闭环与 Windows release 构建通过。真实通用 S3、真实 v1 云库升级/旧权限失效、原生 GUI/安装包和 macOS 保持独立未验收，不将 P6 标为全部完成。各阶段记录保留当时状态，最终结果以文末矩阵为准。

初始工作区已有修改：`docs/README.md`、`docs/s3-backup-implementation.md`、`docs/s3-video-backup-plan.md`；两份 `r2-multi-device-sync-*.md` 为原有未跟踪方案。保留这些内容，本任务只做必要的增量更新。不提交、推送或发布，不运行生产库迁移。

## P1 检查点

当前状态：`CHILD_AGENT / PASSED`，两轮修复后无剩余 P0/P1，可以进入 P2。只代表 P1 检查点，不代表最终交付完成。

独立首审 `CHILD_AGENT / BLOCKED`：旧跨库映射未拆分、半迁移 v2 校验不足。修复轮次 1：迁移复制独立业务记录/文案/缓存并重绑定各自操作，保留远端 ID、对象键和原操作 payload；v2 复用已有来源/容器/大小校验并验证字段及冲突版本。运行服务对未升级空间暂停调度，不直接重放旧队列。新增升级后的外部结果核验：只有精确匹配的旧本机发布证据才能建立上传证明，其余未决队列持久阻断。

修复后 `p1_`：8 passed，包含真正并发迁移、跨库旧映射、损坏半迁移、元数据提交丢响应、分页墓碑和上传证明；桌面全 target Clippy 退出 0。独立复审发现 v3 的 `upload` 也可能只是复用既有视频并发布文案，无法凭这些字段证明实际媒体上传。修复轮次 2：仅核验旧发布结果，不回填视频上传证明，v3 来源保留 unknown。新协议需要专属媒体写证据与活动记录确认后才置真。

- `backup/provider.rs`：提供商中立的作用域、引用、字段补丁、能力、发布回执及业务接口；Telegram 返回 `BACKUP_PROVIDER_NOT_IMPLEMENTED`。
- `backup/sync-schema.sql`、`repository.rs`、`model.rs`：本地 schema v4；事务重建空间角色约束以接受平等成员，保留旧字段与精确目标。增加空间成员映射、基线、局部修订、冲突、上传证明和检查点表。无证据的上传来源保持未知。
- `backup/s3_provider.rs`、`upgrade.rs`：v2 wire 格式与显式、可恢复的元数据升级；原媒体对象不迁移。迁移固定操作 ID；其他发起者拒绝进入。普通保存/连接不调用升级。
- `backup/sync_tests.rs`：迁移、同名远端身份隔离及升级故障续跑测试。

| 实际命令 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml p1_ -- --nocapture` | 3 passed |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --test backup_migration` | 6 passed（角色约束重建前；重建后整轮回归只命中其中 1 项） |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml backup -- --test-threads=1` | lib 43 passed、2 ignored；集成 1 passed |

Windows linker 输出生成 import library 的提示；不属于测试失败。独立审查进行中，后续修改使受影响证据失效时另记验证。

真实 R2 / 通用 S3：`NOT_RUN`。原生 Tauri：`NOT_RUN`。macOS Intel / Apple Silicon：`UNAVAILABLE`。这些状态不能由模拟存储或 Windows 编译替代。Telegram：`NOT_IMPLEMENTED`。

升级边界：旧客户端必须停用，旧写权限撤销或轮换并实测失效，确认旧在途操作结束后才能显式调用升级。中断后沿用同一迁移操作 ID；不以新的操作 ID 接管。进入 v2 后不能只改库版本标志退回 v1；恢复依靠升级前保存的本机数据库及元数据备份，并先停用全部新写者。生产云端升级需要另行授权。

## P2 检查点

状态：`CHILD_AGENT / PASSED`；P2 原 P1/P2 发现已解决。进入 P3。

`S3Provider` 复用现有 `ObjectStore` / AWS SDK，隔离 S3 对象键、ETag 和分片会话。`SyncEngine` 使用三方字段合并、共同基线、有限 CAS 重试、未知提交回读及按字段修订号确认。新增视频按当前空间建立成员关系；远端导入不入队，元数据发布不要求本机视频。操作专属媒体键包括设备与操作 ID，只有自身媒体回执及活动记录收敛后才记录本机上传证明。

连接探针检查条件创建/错误版本拒绝/同版本竞争、分页、Range、分片及清理，不支持条件写时拒绝就绪。S3 新增显式 Region / Path Style 的构造入口；旧 R2 验证入口保留作 v1 兼容测试。

- `cargo test --locked --manifest-path src-tauri/Cargo.toml p2_ -- --nocapture`：6 passed（两独立 SQLite、不同设备及目录；异字段合并、同字段冲突、未知发布、提交期间新修改、条件写能力拒绝、分片取消与续传）。随后补入未知分片创建场景，随整轮备份回归验证。
- `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`：退出 0。

选型：已检索 [OpenDAL](https://github.com/apache/opendal)；现有官方 AWS SDK 和已验证的底层接口足以承载首版，额外存储框架不能替代本项目的资产合并/墓碑/空间语义，故不新增依赖。条件提交语义依据 [AWS 条件写入](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html)，实现使用锁定 SDK 的实际 API，并由模拟探针及后续授权环境验证。

修复轮次 1：独立审查发现整条基线确认会改变上传期间新修改的共同基线；改为持久化逐字段带标签的 Missing/Value 基线，明确区分缺失与 null。引用按 SHA-256/大小/容器判断相同内容，独立位置不制造重复冲突。新增双数据库交错与相同文案内容回归后 `p2_` 8 passed；桌面 Clippy 退出 0。修复前整轮备份回归 54 passed、2 ignored；修复后的受影响用例均通过。

独立复审补充的非阻断 scope 边界也已处理：在内容去重前校验 change、本次与基线媒体引用及冲突引用的完整空间作用域，防止错误空间引用被同内容去重掩盖。新增 `p2_wrong_space_reference_is_rejected_before_equal_content_deduplication`，要求零写入且明确拒绝。

P2 最终增量独立审查 `PASSED`，无剩余发现；上述 scope 用例 1 passed。

## P3 检查点

状态：实施与验收进行中。

- 新增 `backup/spaces.rs`：多空间配置、原子切换/冻结/回切、凭据隔离、能力探测与探针清理日志。旧空间的凭据、操作 ID、原 payload 和资产成员关系保留；保存新空间不补扫历史资产。
- 新增 `backup/sync_read.rs`：统一 Provider 的小对象读取和按需视频恢复；复用媒体头/摘要验证、目标卷暂存、Range、不覆盖提交及共享恢复槽。
- `sync_engine.rs`、`desktop_core.rs`、`desktop.rs`：Rust 服务中独立运行发现与媒体队列；前台 30 秒，失焦/隐藏 180 秒，失败退避；退出取消并加入既有 worker 生命周期。页面仅轮询本机投影与进度。
- `reconcile.rs`、前端 API 与备份 hook：列表按空间成员生成，返回独立墓碑/待提交/冲突/远端修订信息；页面挂载不再触发远端同步，封面按远端修订失效。

实际验证：`cargo test --locked --manifest-path src-tauri/Cargo.toml p3_ -- --nocapture`，6 passed。涵盖 B 不打开任务页且本机上传阻塞时仍发现 A；小对象零视频请求；本机优先、按需恢复及无回声；部分恢复 Range 与同名文件保护；R2/S3 空间切换/凭据及原队列回切；404/分页失败保护；失败探针清理与临时凭据保留及重试。

`cargo check --locked --manifest-path src-tauri/Cargo.toml --features desktop` 退出 0。浏览器与原生运行结果后续独立记录，不以这些 Rust 模拟回归替代。

P3 独立首审发现：活跃探针与后台清理竞争、旧任务文案在重启时回写、旧目标维护阻塞发现/退出，以及旧分页切换取消边界。修复轮次 1：用进程内活跃探针登记隔离本次会话与恢复清理；历史转录仅在资产文案缺失时补种且不登记本地修改；维护独立运行并按自身 journal 记录失败；维护、取消清理、分页及设置请求响应退出取消，空间切换中止旧发现 future。新回归使用真实下载任务与旧转录 snapshot 重启、探针屏障、维护取消、旧分页阻塞及两个库同名远端 ID。修复后 `p3_` 10 passed，桌面 all-target Clippy 退出 0，前端构建退出 0（保留已有第三方警告）。待独立复审。

修复轮次 2：复审定位 `transcription::store` 在 running/interrupted/canceled 等状态中仍会复制旧结果。现在只有真正完成的新本机结果提交才同时更新资产文案；其余状态持久化不调用资产内容写入。`p3_restart...` 扩为旧 running snapshot 重启、开始/取消无新结果，1 passed；现有 `transcription_` 回归 9 passed；桌面 Clippy 再次退出 0。

P3 最终增量 `CHILD_AGENT / PASSED`，无剩余发现，可进入 P4。

## P4 检查点

状态：实现及验收进行中。

新增 `backup/sync_delete.rs`，复用 `backup_operations` 保存当前空间的删除意图、本机清理阶段、墓碑发布及物理清理复核。发起者先清理其明确选定的受管文件；若另一个独立本机资产引用同一路径，保留共享物理文件，仅解除本次关联。失败阻断后续远端删除。

`backup_tombstones` 独立保存权威墓碑；后台仅标记成员已删除/云端隐藏并停止原 ID 上传，保留旧同步投影、本机文件、待提交内容与冲突。实际缺失视频读取复核最新墓碑后，只有本机真实上传证明存在时才清理旧投影及可证明的封面缓存；所有资产行、文案、冲突、未提交变更和来源证明均保留。未上传/未知来源保留投影，按需确认不发送任何远端删除或分片中止。

S3Provider 按精确资产前缀枚举全部对象和分片，包含未在本机清单中的旧版本；先完成分页再删除，避免分页期间移除对象造成漏项。发起者在 30/120 秒后持久复核，三轮后完成；下次启动重新打开原删除操作做一次晚到对象复核，沿用原 ID。其它空间的作业保持暂停。释放本机空间要求无待提交内容、最新活动记录与实际远端媒体大小/摘要可验证，404 不视为删除且不允许丢弃唯一副本。

- `cargo test --locked --manifest-path src-tauri/Cargo.toml p4_ -- --nocapture`：9 passed。
- `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`：退出 0。
- `git diff --check`：退出 0。

九项覆盖 A 墓碑/物理清理、B 本机视频保留、B 未上传与曾上传的后台/按需不同状态、未同步文案、删除与晚到上传交错防复活、未索引旧对象/其它设备分片、同名远端 ID 跨空间隔离、发起者重启续查和释放唯一副本保护。真实远端、原生与跨平台状态仍分别为 `NOT_RUN` / `UNAVAILABLE`。

P4 首审发现释放等待期间状态变化与重复删除日志的活动唯一索引冲突。修复轮次 1：文件 unlink 前在同一 Core/DB 临界段重新验证当前空间、库身份、墓碑、删除意图及未提交内容；重复显式删除复用原 operation；启动仅为无活动删除的成员重开最新一条已完成操作，保留其余历史。新增写锁屏障下的墓碑/切换/删除意图测试、两个 completed 及 completed+active 的重复启动和重复显式删除测试。修复后 `p4_` 12 passed，桌面 Clippy 退出 0。修复前整轮备份 76 passed、2 ignored，修复后受影响 12 项通过。独立复审 `CHILD_AGENT / PASSED`，进入 P5。

## P5 检查点

独立审查修复后 `CHILD_AGENT / PASSED`。设置、API 类型、上传/删除菜单移除日常 primary/reader；旧适配器保留用于迁移和回归，不作为生产 v2 worker。

- `backup-settings.tsx` / `api.ts`：R2/通用 S3、多个已保存空间、回切、新建草稿、Region/路径样式、Telegram 禁用、凭据掩码、测试和保存反馈。目标编辑进入新空间并清密钥，避免新密钥被错误发送到原升级目标。
- `conflicts.rs` / `backup-conflicts.tsx` / `provider.rs` / `sync_engine.rs`：读取独立版本，持久选择通过正常条件提交；只消除精确版本，并发新版本保留。`export_data` 导出实际 pending 字段/选择、完整 ConflictVersion、本机文案与身份，避免导出 UI 占位文本。
- `spaces.rs` / `upgrade.rs` / `desktop.rs`：独立升级入口，用户确认外部旧客户端/权限已隔离并填写新凭据；能力探测在升级前，v2 协议/能力/凭据在对账后一起启用，普通保存不能绕过旧库升级。
- `backup-tasks.tsx` / `main.tsx` / `video-transcript.tsx`：当前空间状态、冲突、本机待处理；已有任务释放副本后从播放/转录走资产懒恢复，读取规范资产文案，不优先展示旧 task 快照；小对象不触发视频恢复。
- `s3_provider.rs` / `sync_engine.rs`：真实已消费字节和已确认分片展示进度；确认收敛之前最多99%，没有虚构填充；续传检查点不被后续阶段的旧 payload 覆盖。
- 更新 `DESIGN.md`、[使用维护](multi-device-sync-operations.md) 与开发入口。

| 命令/专项 | 实际结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml p5_ -- --test-threads=1` | 3 passed |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml backup -- --test-threads=1` | lib 82 passed、2 ignored；匹配集成 1 passed |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | exit 0 |
| `npm run build` | exit 0；保留既有 use-client/sourcemap、chunk 体积警告 |
| `playwright-cli --session p5-sync run-code --filename tests/s3-backup-browser.mjs` | PASS；3视口、列表/信息流、实际进度、取消、Tooltip/键盘、掩码、保存不闪、平等设备操作 |
| `playwright-cli --session p5-spaces run-code --filename tests/multi-device-sync-browser.mjs` | PASS；3视口、6次空间切换、冲突采用、本机文案导出、释放后的播放、Telegram禁用、错误升级目标阻断 |

浏览器模拟 IPC，无生产用户文件或云端请求。截图：`output/playwright/`。P5 首审发现的目标/凭据错配、导出缺内容、已释放任务缺懒播放入口均已修复并有回归断言。初次新增资产匹配把空 selected 与空 task_id 匹配导致弹窗误开，已以非空选择门禁修复，浏览器重跑通过。

## P6 集成验收

新增 `sync_online_tests.rs`：默认 ignored，R2 使用私有配置标记 `environment=dedicated-test` 的写凭据，通用 S3 使用独立私有配置。每次生成授权前缀内的 `v2-run-<UUID>/`，只处理该 run 对象和会话；使用临时受控容器样本，不使用用户视频。两个独立 SQLite、不同设备 ID、不同本机目录和不同本机 space_id 共用相同远端 library_id。脱敏身份和前缀记录于忽略目录 `.local/sync-v2-test-runs.jsonl`。

真实 R2 验收中发现创建和列举返回不同文本的 UploadId。原探针错误地要求字符串一致而拒绝连接。修复为验证列举出的操作专属 key 唯一会话能读取同一分片的长度/ETag，并用该列举句柄实际完成分片，随后验证内容和清理；仍严格验证竞争条件写拒绝及 Range，不降低多写门禁。相关官方 API 兼容信息见 [Cloudflare S3 API](https://developers.cloudflare.com/r2/api/s3/api/)，别名行为的结论来自本轮实际请求，不由文档推断。

| 已运行项目 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml p6_real_r2_v2_ -- --ignored --nocapture --test-threads=1` | 2 passed，exit 0，96.33s；含100 MiB、丢首分片响应后实际关闭数据库/重开、同 operation、parts 1–7各一次 |
| 扩展后 `... p6_real_r2_v2_two_devices -- --ignored --nocapture --test-threads=1` | 1 passed，exit 0，38.13s；补文案回传/无视频重传、浏览无视频 GET、冲突读取/采用/收敛；listed ID 实际 CompleteMultipart |
| 每次在线 run 清理 | 全部本轮前缀对象及分片会话为空，包含失败探针的 run；未列举/删除授权前缀外目标 |
| 审查侧 Accepted 与探针定向用例 | 各1 passed，exit 0；Accepted 不推进 ack/proof，不清 pending |
| `tests/transcription.browser.js` | 5 passed：3视口的状态、复制/导出/取消、显式模型启动、缺源保留文案 |
| `tests/transcript-overflow.browser.js` | 6 passed：3视口×长文/连续字符、内部滚动/焦点/固定操作 |
| `tests/startup-paint-browser.mjs` | PASS：3视口、JS前首屏、React接管、延迟说明块、GFM、pageErrors=0 |

启动回归原先硬编码1421端口，指向 dev 服务时未经过打包 chunk，修复为使用当前预览源，在1422的 dist 预览通过。一轮 Rust 重跑与尚在运行的同名测试 EXE 链接并发，出现 LNK1104；等待前一进程结束后顺序重跑通过。全量回归新增文案冲突采用后重启测试发现：确认已清除的引用修改仍遗留旧 reference_bases，使采用被再次判成冲突。修复为仅随对应已确认修订一起清除其引用基线，较新本机修订的基线继续保留；采用时依据最新远端记录丢弃该字段的历史编辑基线。审查同时修复手动请求被未决旧协议操作（含 canceled）或删除意图阻挡时仍提示排队成功的问题：保留原 payload，明确报需复核/已删除，不重放旧会话。修复后 P6 定向4 passed、3在线 ignored，最终全量通过。

### 最终命令和结果

| 实际命令 | 最终结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` | exit 0；lib **221 passed、0 failed、7 ignored**，208.07s；迁移集成 **6 passed**，0.51s；bin/doc 0 tests |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml p6_ -- --test-threads=1` | exit 0；**4 passed、3 ignored**，0.50s |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | exit 0，24.92s |
| `npm run build` | exit 0，含 TypeScript 检查与 Vite 构建 |
| `npm run desktop:build` | exit 0；现有引擎预检通过（0.14.0）；Tauri Windows release 构建通过，1m49s，`--no-bundle` |
| `git diff --check` | exit 0，包含新文件的额外空白检查和文档链接检查 |

默认7 ignored 为5个须显式运行的云端用例（其中新版 R2 的2项已另行运行）与2个既有 GitHub 在线更新用例；不是7项失败。最终默认回归保留这些 opt-in 门禁。原一轮全量结果为219 passed/1 failed/7 ignored，失败为上述引用基线缺陷；修复后的221 passed/0 failed包含新增旧操作排队反馈测试，不弱化原断言。

原始本机日志均在忽略目录：`.local/sync-final-cargo-fixed.log`、`sync-p6-final.log`、`sync-final-clippy.log`、`sync-frontend-build.log`、`sync-desktop-build.log`、`sync-r2-final.log`。浏览器记录以对应 run-code 的 PASS / passed 结果为准，不以 CLI 包装命令退出0代替页面断言。

Windows 产物：`src-tauri/target/release/yingqu.exe`，48,967,680 bytes，SHA-256 `B9EEF0159E43EB80D1A3BD06126A75A06A2EF0A6D746989C4FBE3F1FBE97D213`。构建目录保留 `video-engine/` 运行资源；不发布、不安装、不覆盖用户安装版。

### 行为与环境矩阵

| 项目 | 离线/浏览器 | 真实环境 |
| --- | --- | --- |
| 两套 SQLite、不同设备/目录、双方新增下载 | PASS；独立来源证明、无导入上传回声 | R2 双库/设备 PASS；双方各新增的真实并发场景 NOT_RUN |
| 不同字段合并、同字段冲突/采用 | PASS；含文案引用、选择后数据库重启 | R2 标题/作者并发、冲突采用 PASS；真实并发转录冲突 NOT_RUN |
| 文案独立发布、A读取B文案 | PASS；无视频恢复/重传，未提交文案保留 | R2 PASS，视频 stream/write 计数明确为0 |
| 断线、提交丢响应、取消归属 | PASS；可控故障、CAS有限重试、各自会话 | R2 100 MiB丢首分片响应、真实关闭DB重启/续传 PASS；真实双上传取消和CAS丢响应 NOT_RUN |
| 墓碑、按需收敛、防复活 | PASS；有文件/无文件/上传证明unknown或true、404不删除、删除交错 | R2 墓碑、本机文件/未同步文案保留和发起者清理 PASS；真实交错组合 NOT_RUN |
| R2/S3空间切换/回切、同远端ID隔离 | PASS；原操作和payload恢复，不迁移历史，不跨目标写 | 真实双空间切换 NOT_RUN（无授权通用S3配置） |
| S3缺原子写入的拒绝 | PASS；探针不产生虚假就绪，失败清理可恢复 | R2 CAS探针 PASS；真实非原子服务 NOT_RUN |
| v3→v4本地、v1→v2云端升级 | PASS；旧库事务迁移、半完成恢复、两发起者互斥、无凭空上传证明 | 真实 v1 副本升级/凭据撤销 NOT_RUN；生产库未迁移 |
| Telegram错误启用 | PASS；UI禁用，后端拒绝，零网络/队列伪成功 | NOT_IMPLEMENTED，首版明确排除 |
| Windows构建与界面 | release构建 PASS；5组浏览器回归 PASS，三视口 | 原生GUI/托盘/安装包验收 NOT_RUN，未在隔离原生用户目录启动 |
| macOS Intel / Apple Silicon | UNAVAILABLE | UNAVAILABLE，未提供对应运行/构建环境 |

删除的六种状态分别验证：A 持久意图及受管文件处理；远端库 tombstone 长期存在；B 缺文件且无待处理数据时从云端列表隐藏；B 本机资产记录继续保留；B 有效本机视频继续可用；B 未提交文案/标题/冲突及来源证明不清除。发起设备清理远端物理对象；B 不注册跨设备删除任务。模拟用例另覆盖同名资产在其它空间的墓碑、对象和队列不受影响。在线测试的最终 run 清理是测试夹具专用清理，不是产品允许其它设备接管删除。

没有改变下载引擎、转录引擎或平台协议。全量 Rust 的队列、下载、封面、文件提交、取消/退出、历史与转录回归均通过；浏览器转录和启动专项通过。没有使用生产凭据/视频，也没有执行生产云库升级、提交、推送、发布或安装工具。


## 删除记录定期清理与对象类型（2026-10-31）

本节记录新增保留策略与Content-Type的当前验证，以上P1–P6在线记录为历史证据，不代表本次清理功能已经做过真实R2验证。未使用生产凭据、视频或Bucket；保留已有工作区修改，未提交、推送、发布或安装。

### 实现与兼容

远端备份按空间保存“自动清理已删除记录”，默认关闭、30天，允许1～3650整数。仅当前启用且就绪的空间由Rust原维护循环处理，正常每小时检查，失败保存原因并5分钟后重试。每批最多确认128条合并；停用、退出或切换空间暂停，回切恢复。只处理已确认的过期墓碑，活动记录不按时间删除。

先CAS合并`deletions/<UUID前两位>.json`精简索引，确认持久后按旧ETag条件删除独立`records/<asset_id>.json`；活动记录变化、索引失败或身份不符均不会导致无条件删除。服务不支持`DeleteObject If-Match`时明确拒绝清理，通过隔离探针验证错误版本拒绝、正确版本可删除，不影响未启用清理的普通备份。没有可靠删除时间的记录先持久写入起始时间，后续不重置。

有效本机副本可显式重新上传。索引不限制上传次数，重新上传及后续封面/文案使用该资产删除修订下的不可变路径；旧清理不重发墓碑，不删除新对象或中止新会话。普通上传跨再次删除后过期，显式重试新建操作，保留旧会话清理证据。未确认发布的冲突存于`PendingChanges.retained_conflicts`，在墓碑、活动发现、数据库重启和重试中保留；Converged仅确认本次Capture所带版本，晚到版本继续待处理。缺失冲突文件保留完整引用/导出，不能采用；仍存在的冲突内容采用时重新上传至当前路径。

普通PUT、文件PUT和CreateMultipart均设置容器对应的Content-Type：JSON `application/json`，MP4 `video/mp4`，MOV `video/quicktime`，MKV `video/matroska`，WebM `video/webm`，JPEG/PNG/WebP为相应`image/*`。无格式探针保留`application/octet-stream`，历史远端对象不自动改写。

SQLite沿用`backup_schema=4`的幂等初始化，新增`backup_record_retention`，按space_id保存enabled/days、next_check_at/last_error、delete_verified；已有空间默认关闭。PendingChanges新增字段采用serde默认，旧JSON可读；不迁移视频、凭据、历史资产关联或旧队列。主要接口见[维护说明](multi-device-sync-operations.md)，表单交互见[设计规范](../DESIGN.md)。

### 当前命令与实际结果

| 实际命令 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml retention_ -- --test-threads=1` | 19 passed，0 failed，3.13s |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml content_type_tests -- --test-threads=1` | 2 passed，0.03s；localhost HTTP fixture核验SDK真实请求头，亦包含于最终全量 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` | lib 261 passed、0 failed、7 ignored，201.02s；迁移集成6 passed，0.51s；bin/doc 0 tests |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | exit 0，4.51s |
| `cargo doc --locked --manifest-path src-tauri/Cargo.toml --no-deps` | exit 0，4.09s |
| `npm run build` | TypeScript与Vite构建通过；保留既有use-client/sourcemap及chunk体积警告 |
| `playwright-cli --session record-retention-final run-code --filename tests/backup-settings-actions-browser.mjs` | PASS；3视口，启停/连接/重试/删除/持久错误轮询/过渡动画 |
| 同session `run-code --filename tests/backup-space-tabs-browser.mjs` | PASS；3视口，内联改名/草稿隔离/保存不激活/默认门禁/保留策略校验 |
| 同session `run-code --filename tests/multi-device-sync-browser.mjs` | PASS；3视口，6次切换/冲突采用/导出/懒播放/本机副本重新上传/Telegram禁用 |
| 同session `run-code --filename tests/settings-layout-browser.mjs` | PASS；1440×900、1280×800、1024×680，各3轮切换，严格CSP与内部滚动 |
| 同session `run-code --filename tests/s3-backup-browser.mjs` | PASS；3视口，列表/信息流/上传恢复进度，浏览视频请求为0 |

浏览器为隔离IPC fixture，不替代原生Tauri。Rust使用至少两个独立SQLite、设备ID与本机目录；冲突发现回归使用3设备，明确验证“未见旧冲突的C先重新上传→B先拉取→B重开数据库并重试→原版本仍可详情/导出”。另外覆盖年龄边界、无时间墓碑、同分片CAS并发、写索引失败、删除响应未知、非原子删除拒绝、晚到上传/后续转录封面保护、来源证明、空间切换取消/恢复、保留未同步文案与晚到冲突确认。

初期一轮全量为253 passed/1 failed/7 ignored，另一轮为259 passed/1 failed/7 ignored，失败均为既有`tdl_bridge::tests::deferred_channel_waits_for_telegram_but_allows_other_platform_and_keeps_fifo`读取夹具`events.jsonl`遇EOF；本次未改Telegram源码或弱化该断言，两轮中间全量与当前最终全量均通过。该夹具存在间歇失败，保留失败日志供后续独立排查。审查补充的活动发现断言在修复前确实失败（`round4-reproduction.log`）；修复后同一断言及19专项通过。扩展并发测试曾因新增合法标题冲突而与原“两项完全相等”预期不符，已更新为原两项逐一原样存在并额外保留合法新冲突，共三项，不减少保留断言。

### 当前环境边界

真实R2与通用S3清理/MIME：NOT_RUN；条件删除兼容性由实际保存/后台隔离探针验证，不以mock结果代替。原生Tauri界面、托盘、安装包验收：NOT_RUN。macOS Intel/Apple Silicon：UNAVAILABLE。Telegram真实存储仍NOT_IMPLEMENTED，不能误启用。本次不启动生产配置，不覆盖用户安装版；Windows正常EXE的最终构建、SHA和审查冻结记录在下方产物段。

### 本次Windows产物及审查

`npm run desktop:build` exit 0，正常Windows release EXE构建通过，Rust release增量构建43.66s，含TypeScript/Vite及既有引擎资源预检（0.14.0）；`--no-bundle`，未生成安装包。保留既有STATIC_VCRUNTIME弃用和链接器提示。产物仍为`src-tauri/target/release/yingqu.exe`，49,731,584 bytes，SHA-256 `4C37079590F7E63F84417DE07C6BE9DDF6BADFFFF1098224F5E517DF53E9AF94`；未生成yingqu-fixed.exe，未覆盖用户安装版，未在生产配置下启动。

`git diff --check`通过；新文件另做新增行空白检查。独立CHILD_AGENT以FULL_SCOPE覆盖25个实现与配套路径，第4轮源码复核PASSED，未见剩余P0/P1；用户明确追加最多3轮，当前累计4/6轮。实现冻结清单`.local/record-retention/review-target-repair4.json`，SHA-256 `2BB438D197E0B7C8812E8F0F02A4625C00014A79A1DAC74B15A095669DA9541B`。最终源码、验证文档及EXE指纹汇总在`.local/record-retention/review-target-final.json`，验证明细在同目录`verification.md`。

最终原始日志：`round4-final-targeted.log`、`round4-full.log`、`round4-clippy.log`、`round4-doc.log`、`round4-desktop.log`，以及`browser-*-final.log`、`content-types.log`。历史失败及修复前回归仍保留，不用最终结果覆盖失败证据。


## 个人备份浅层目录（2026-10-31）

本节为后续独立交付：用户明确不要求保留多设备同时上传和上传断点续传，以同视频与封面共用浅层目录为目标。前面多设备/续传验收为历史结果，不再作为新媒体上传的支持承诺。

### 当前行为与兼容

新视频、封面和文案统一在`<Prefix>media/<asset_id>/`：`video-<SHA256>.mp4`、`cover-<SHA256>.png`、`transcript-<SHA256>.json`。删除后重新上传只在文件名插入`r<删除修订>-`，不增加目录。设备ID和操作ID不进入对象键；文件内容SHA256仍进入文件名，防止小对象发布失败覆盖仍被旧记录引用的内容。

S3Provider不提供断点续传（`resumable_upload=false`）。大文件仍流式分片；每次失败/重启后重试先清理旧会话，从新会话的第1片开始，不读取或复用已有分片。已提交且长度/SHA256一致的完整对象直接确认。未知CreateMultipart响应、普通取消及旧深层键重试到新键均按已校验的精确键清理，失败保留证据并明确报错。当前按单人单写使用，不保证多设备并发媒体上传，不新增远端锁或第二套任务。

历史深层对象按原记录继续读取、获取封面/文案、按需恢复和删除；不批量迁移、改写或删除生产对象。新配置/现有空间的身份、凭据、资产关联与待办保留；SQLite表结构和记录版本不变，既有私有状态仅用于清理未完成会话，不作字节续传。MIME设置继续覆盖普通PUT、文件PUT和分片创建。设置页概括文案改为“备份视频与文案，视频按需下载。”。

### 已完成的验证

| 实际命令 | 当前结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml flat_ -- --test-threads=1` | 6 passed，0 failed，7.67s；同目录、同内容复用、新旧路径恢复、失败发布保留旧文件、未知创建取消和整次重传 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml retention_ -- --test-threads=1` | 19 passed，0 failed，3.34s；此组亦包含于最终全量 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` | 最终lib 265 passed、0 failed、7 ignored，177.29s；迁移6 passed，0.51s；bin/doc 0 tests |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | exit 0 |
| `cargo doc --locked --manifest-path src-tauri/Cargo.toml --no-deps` | exit 0 |
| `npm run build` | TypeScript/Vite exit 0，9.24s；保留既有use-client/sourcemap与chunk体积警告 |
| `playwright-cli --session flat-backup-final run-code --filename tests/backup-settings-actions-browser.mjs` | PASS，3视口，新的概括文案、启停/保存/默认/删除和错误反馈，pageErrors=0 |
| `playwright-cli --session flat-backup run-code --filename tests/settings-layout-browser.mjs` | PASS，1440×900、1280×800、1024×680，各3轮切换，严格CSP、滚动和键盘，pageErrors=0 |

Multipart测试明确记录part序列`[1,1,2,3,4,5,6,7]`，重试使用新upload_id、旧parts读取次数为0；不是把完整对象的幂等确认当作续传。新旧路径恢复均验证实际视频文件内容，列表/封面/文案在恢复前视频请求为0。发布失败时远端旧记录、封面和文案内容仍可读，重试后新引用可用；删除覆盖同资产全部新旧对象，不删除其它空间。

早期备份回归125 passed/2 failed/5 ignored：旧`transcripts/`目录断言和续传状态期望与新要求冲突，已改为浅层对象是否存在/能清理，以及实际新引用与正文是否正确；没有减少删除或正文保留断言。新增3项初期因夹具未先生成指纹，文案无内容身份关联而失败，补齐有效夹具后通过。浏览器初期等待已取消的多设备文案而超时，已同步新文案并保留其余交互断言。审查确认的未知创建取消残留P2已修复并补回归。上述中间日志保留在`.local/flat-backup/`，不是最终通过结果。

实现与测试指纹冻结于`.local/flat-backup/review-target-repair1.json`，SHA256 `29170BE828184EEEB3EE65DF9260CB15FA11E8DB550EA5C3CDB464A64ACCDC50`；独立CHILD_AGENT以FULL_SCOPE完成15路径复审，P2已解决，无剩余P0/P1/P2。实际源码未扩展为全量移除元数据发现/历史兼容，后台队列、发布确认、按需恢复、空间隔离及本机未提交数据保护继续复用。

### 本次边界

真实R2/S3：NOT_RUN；未读真实测试或生产凭据，未触碰用户F盘视频和生产Bucket。原生Tauri功能、托盘、安装包验收：NOT_RUN；只读检查运行进程及窗口清单不算原生功能验证。macOS Intel/Apple Silicon：UNAVAILABLE。未提交、推送、发布、安装或启动新EXE的生产配置。正常Windows产物和最终冻结信息见下方；仍不生成yingqu-fixed.exe。

### 浅层目录交付产物

`npm run desktop:build` exit0，含TypeScript/Vite、引擎0.14.0预检，Rust release增量构建49.47s，`--no-bundle`。正常产物`src-tauri/target/release/yingqu.exe`，49,743,360 bytes，SHA256 `041ED8B2E32355E90E5ED8A0A3E0D905FCA49DA8E8E2792F19F3671730A796B9`；未改名为fixed、未安装或发布。构建前确认工作区进程已退出，未启动新EXE验证生产配置。保留既有STATIC_VCRUNTIME弃用、链接器及Vite体积提示。

`git diff --check`与新增行空白检查通过。当前任务从独立基线开始，保留之前全部已有修改；15路径实现/契约代码审查通过，补入本节验证记录后最终16路径及EXE/证据指纹见`.local/flat-backup/review-target-final.json`。详细验证和范围说明为`.local/flat-backup/verification.md`；原始最终日志包括`flat-repair-tests.log`、`full-final.log`、`clippy-final.log`、`rust-doc-final.log`、`desktop-final.log`、`frontend-final.log`、`browser-actions-final.log`、`browser-layout.log`。
