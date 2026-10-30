# S3/R2 备份实施验证记录

实施基线：`codex/s3-video-backup` / `2d32959`。当前任务开始时，`docs/README.md` 已修改，两个 S3 方案文档未跟踪；这些用户文档保持原样。

## M1 资产模型与读写角色

状态：M1 已通过离线验收及独立 CHILD_AGENT 复审，进入 M2。

### Files / Scope

- 新增 `src-tauri/src/backup/model.rs`、`repository.rs`、`schema.sql`、`store.rs`、`mod.rs`。
- 修改 `src-tauri/src/desktop_core.rs`、`lib.rs`、`transcription.rs`。
- 新增 `src-tauri/tests/backup_migration.rs`。
- `.gitignore` 增加用户要求的根目录私有测试配置 `s3-r2-test.local.json`；配置文件只留本机，不提交凭据。

### Contracts

- 六张方案表及设备身份、迁移版本、独立封面存储。
- `Task.asset_id` 向后兼容；完成视频与资产关联同事务保存，图集和未完成任务排除。
- 资产转录独立保存；历史清理解除任务关联，保留视频、资产和文案。
- 摘要未验证的旧转录保留可读文本，摘要为空时不能用于远端发布或内容复用。
- 库创建使用条件写入并回读；每次普通写操作验证角色、设备及库世代。恢复设备写调用在到达传输前拒绝。
- 备份空间身份不可原地改向；不确定的运行中操作恢复为待核验，同时保留原阶段和操作内容。

### Tests

| 命令 | 实际结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml backup:: -- --nocapture` | 退出 0；2 个权限和库身份测试通过 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --test backup_migration` | 最初测试夹具未符合已有平台身份校验，修正夹具后 3 项通过；新增第 4 项随下面全测通过 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1` | 首轮 134 通过、1 失败、2 忽略；新增摘要任务延长下载占用，历史移除返回 `TASK_BUSY` |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml author_avatar_survives_restart_and_reads_only_task_owned_cache -- --nocapture` | 调整摘要执行位置后退出 0；原失败回归通过 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` | 退出 0；lib 135 通过、2 忽略；迁移集成 4 通过；文档测试 0。忽略项为原有 GitHub 在线测试 |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | 退出 0 |
| `git diff --check` | 退出 0 |

### Review / Risks

独立 CHILD_AGENT 首轮发现摘要退出取消缺口；已接入独立 watch 取消信号并按块检查，取消不标记损坏。复审 PASSED，无剩余 P0/P1。

修复后 `cargo test --locked --manifest-path src-tauri/Cargo.toml shutdown_cancels_an_in_progress_asset_fingerprint -- --nocapture` 退出 0（1 通过），迁移集成 4 通过，桌面 Clippy 与差异检查均退出 0。

M1 检出的启动同步摘要 P2 已在 M2 移入启用备份后的后台队列。

R2：`NOT_RUN`，尚未使用任何真实凭据或远端对象。

Native：Windows 原生 Tauri `NOT_RUN`；macOS Intel / Apple Silicon `UNAVAILABLE`。当前仅 Windows Rust 测试和桌面编译静态检查，不能替代原生或安装包验收。

## M2 S3 客户端、设置及自动上传

状态：离线实现及独立 CHILD_AGENT 复审 PASSED，进入 M3。真实 R2 当前 FAIL（403）。

### Files / Contracts

- 新增 `backup/s3_client.rs`、`settings.rs`、`record.rs`、`scheduler.rs`、`tests.rs`，扩展 `store.rs`、`repository.rs`、`schema.sql`、`mod.rs`。
- 修改 `Cargo.toml` / `Cargo.lock`：官方 aws-sdk-s3 1.156，关闭默认功能，启用 Tokio/TLS/HTTP，不启用 SigV4a；Windows Rust 1.98 编译验证。
- 修改 `desktop_core.rs`、`desktop.rs`、`src/api.ts`：独立后台 worker、退出取消、脱敏设置、连接检测、操作查询/取消/重试；迁移测试改为显式异步摘要，启动不再读取整个视频。
- 系统凭据引用轮换与延后清理，数据库不存密钥；同一空间离线关闭不要求凭据或远端连接。
- 单上传并发、持久阶段、100 MiB 分片阈值、16 MiB 初始分片、ListParts 续传、未知创建及完成结果核验、源变更重启、随机指数退避及有界尝试。
- 专用写探针先持久清理意图；恢复设备连接检测只有读取。取消清理限定资产媒体键，停用旧空间不会阻塞当前队列。
- 经用户确认，修订主方案和任务文档：封面/转录额外使用自身 SHA-256 路径，不覆盖旧 record 引用对象；写对象键逐项持久化，供后续删除全部版本。

### Tests / Review

- `cargo test --locked --manifest-path src-tauri/Cargo.toml backup -- --test-threads=1`：首轮 9 项通过；增量旧空间取消测试 1 项通过。
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --test backup_migration`：4 项通过，退出 0。
- `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`：退出 0。
- `npm run build`：退出 0，保留已有第三方 use-client/sourcemap 和 chunk-size 警告。
- `git diff --check`：退出 0。
- 独立首审发现旧空间取消清理饥饿、固定小对象路径覆盖问题及离线关闭问题；已修复并增加回归，冻结目标独立复审 PASSED，无未解决 P0/P1/P2。

新增完整性回归首次失败，定位为已验证资产文本缺少本地 source_json 时摘要被错误清空；改为内容摘要优先保留，针对性重跑通过。12 项离线测试均已通过（11 项整轮、最后修复 1 项单独重跑），真实测试 1 项默认 ignored；修复后迁移 4 项与 Clippy 再次退出 0。

### R2 / Native / Risks

`cargo test --locked --manifest-path src-tauri/Cargo.toml real_r2_m2_dedicated_prefix -- --ignored --nocapture`：退出 101。配置中的测试 Bucket 内随机 run 子前缀，首个 library 创建和随后该前缀 List 均返回 403；未获得任何写入成功响应，也无法核验远端清理终态。已请求用户核查 R2 S3 Access Key/Secret 和 Bucket 权限；不将拒绝请求记作上传/恢复通过。后续测试 run 身份记录于忽略目录 `.local/r2-test-runs.jsonl`，不含密钥。

Windows desktop 编译静态检查 PASS；原生 Tauri、安装包 NOT_RUN；macOS Intel/Apple Silicon UNAVAILABLE。真实 R2 待凭据纠正后复验。

## M3 只读发现与文案

状态：离线验收与独立 CHILD_AGENT 复审 PASSED，无剩余 P0/P1/P2，进入 M4。

Files：新增 `backup/reconcile.rs`、`src/components/cloud-backup.tsx`、`tests/s3-backup-browser.mjs`；修改 `backup/record.rs`、`mod.rs`、`tests.rs`、`desktop.rs`、`transcription.rs`、`src/api.ts`、`src/main.tsx`、`DESIGN.md`。

Contracts：分页/ETag 增量导入，畸形记录拒绝，断网保留稳定列表，不创建伪下载任务；墓碑保留本机视频；封面按可见区域读取，文案独立读取/复制/导出，均不恢复视频。已有本机生成文本优先，且远端返回后落库再次保护本机新转录，已缓存文本离线可读。

Tests（均退出 0）：`cargo test --locked --manifest-path src-tauri/Cargo.toml reader_discovery -- --test-threads=1` 1通过；`remote_record_validation` 1通过；`reading_old_remote` 1通过；同命令筛选 `export` 3通过；desktop all-target Clippy；`npm run build`；`git diff --check`。

Browser：`playwright-cli --session s3-backup run-code --filename tests/s3-backup-browser.mjs` PASS。1440×900、1280×800、1024×680无水平溢出，搜索、文案Dialog/Escape、断网保留列表、持续错误通过，0视频恢复请求、1文案请求，无pageerror。初次测试mock缺少事件注销API，修正测试环境后通过；不修改生产行为绕过。截图检查保存在忽略目录`.local/s3-m3-1024.png`。

Review：修复旧远端文本覆盖本机新文本及封面刷新失效，独立复审通过。

R2：待凭据修正，仍为 M2 403 FAIL。Windows 原生 Tauri NOT_RUN；macOS两架构 UNAVAILABLE。

## M4 按需恢复与本机访问

状态：离线验收及独立 CHILD_AGENT 复审 PASSED，无剩余 P0/P1/P2，进入 M5。

Files：新增 `backup/restore.rs`；修改 `backup/repository.rs`、`scheduler.rs`、`mod.rs`、`tests.rs`、`desktop_core.rs`、`transcription.rs`、`desktop.rs`、`src/api.ts`、`video-player.tsx`、`video-transcript.tsx`、`cloud-backup.tsx`、`playback-position.ts`、浏览器测试。

Contracts：同资产恢复共享、单恢复槽、本机有效文件零网络且不等待远端槽；同盘暂存、Range/ETag 续传、完整摘要及媒体头检查、不覆盖提交、提交意图崩溃恢复；关闭播放器忽略迟到结果，取消/退出/重启复位恢复状态；复用现有 ASR，同内容新路径重新绑定文案。

Tests：`cargo test --locked --manifest-path src-tauri/Cargo.toml restore -- --test-threads=1` 9通过；相同命令筛选 `cancel_restore` 1通过；真实 drop/reopen 数据库后续传 offset `[0,4096]`。desktop all-target Clippy、`npm run build`、`git diff --check` 均退出0。浏览器三视口、文案零视频读取、关闭后迟到恢复均PASS，无pageerror。

Review：修复本机访问等待远端槽、文案源路径重绑定和取消后状态残留；独立复审 PASSED。

R2：仍为403 FAIL。原生播放和真实 ASR 模型 NOT_RUN；macOS两架构 UNAVAILABLE。

## M5 生命周期与界面

状态：离线实现及独立 CHILD_AGENT 复审 PASSED。随后按用户新指示调整任务列表展示，增量验收见下。

Files：新增 `backup/lifecycle.rs`、`src/components/backup-settings.tsx`；修改 `backup/scheduler.rs`、`restore.rs`、`mod.rs`、`tests.rs`、`desktop_core.rs`、`desktop.rs`、`src/api.ts`、`main.tsx`、`task-more-menu.tsx`、`cloud-backup.tsx`、浏览器测试和 DESIGN。

Contracts：持久删除意图先于文件/S3操作，取消上传/恢复/ASR；本机身份检查失败阻止远端操作；离线删除保留 pending_delete，条件墓碑先于精确对象/分片清理。不可变小对象的全部已知版本逐项清理；墓碑永久保留。释放仅本机，不写远端；保留历史和文本。设置支持系统凭据、主/读者角色、离线关闭；原任务删除接入主设备删除协议，列表提供取消、重试及持续错误。

Tests：`cargo test --locked --manifest-path src-tauri/Cargo.toml backup::tests -- --test-threads=1` 26通过/1ignored，退出0。包括断网删除/墓碑后清理失败、身份变更阻止删除、读者零写、删除阻止旧上传复活、数据库真实重开继续删除、只清理自身part和旧文本全部版本。

`npm run build`、desktop all-target Clippy、`git diff --check`退出0。浏览器三视口PASS，设置保存/清空凭据、reader隐藏远端删除、恢复取消、pending_delete持续错误及重试均通过，无pageerror，截图`.local/s3-m5-1024.png`。首次构建引用不存在的Switch，已改复用已有Checkbox；首次浏览器断言遗漏作者后缀，修正选择器后通过。

R2：仍为403 FAIL；原生Windows NOT_RUN，macOS两架构UNAVAILABLE。

### 用户确认的任务列表调整

用户取消独立“云端备份”入口，备份只在设置配置；任务行用云朵填充进度，完成为绿色对号。用户明确同步填充图标仅用于 S3 恢复，不改变普通平台下载。

新增 `backup-tasks.tsx` 替代并删除本次新建的 `cloud-backup.tsx`，修改 TaskList/main/style，将远端资产以独立类型合并展示。新增本机历史隐藏表（backup schema 3），避免清理历史后资产在同一列表重新出现；不改变远端协议版本。设置启用后自动发现清单，轮询只读取本机数据库。

上传进度复用官方 SDK `SdkBody::map_preserve_contents` + `http-body-util::BodyExt::map_frame` 统计实际传输字节。http-body-util 原已为传递依赖，现显式声明；不修改数据、内容长度与重试能力。依据：[SDK 官方源码](https://github.com/awslabs/aws-sdk-rust/blob/main/sdk/aws-smithy-types/src/body.rs)。单文件和分片统一通过持久操作快照更新，record 发布前不显示完成。

浏览器三视口、列表/信息流两视图PASS，上传50%、恢复25%→75%、取消、绿色完成、只读删除隐藏、独立文案和关闭迟到播放均通过。截图 `.local/s3-task-icons-1024.png`。`npm run build`退出0。

增量独立复审 PASSED：修复封面刷新失败后的重试与可见错误，以及平台/搜索过滤后的资产计数；恢复中资产计入下载中。对应浏览器回归通过，无 pageerror。普通平台下载进度未改变。

## M6

最终完整回归：`cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` 169通过/3ignored，迁移6通过，退出0（`.local/s3-final-tests.log`）。之后仅提取调度失败持久化函数，补充 `cargo test --locked --manifest-path src-tauri/Cargo.toml retry_policy -- --test-threads=1`：1通过，退出0（`.local/s3-m6-retry.log`）。首次增量编译因前一轮 Windows 测试进程仍占用可执行文件发生 LNK1104；等待其完成后顺序执行通过。

最终 `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`、`npm run build` 均退出0；日志分别为 `.local/s3-final-clippy.log`、`.local/s3-final-build.log`。浏览器验证命令为 `playwright-cli --session s3-backup run-code --filename tests/s3-backup-browser.mjs`，证据见 `.local/s3-task-icons-browser.log`。

最终源码 `npm run desktop:build` 退出0，生成 `src-tauri/target/release/yingqu.exe`，日志 `.local/s3-final-desktop-build.log`；未生成或验收安装器。`git diff --check` 及21个新增文件空白检查通过（Markdown双空格换行保留）；私有测试配置确认被 Git 忽略。

### M6 故障与环境证据

Files：扩展 `backup/tests.rs`、`backup/s3_client.rs` 的 HTTP body 进度测试、`tests/backup_migration.rs`、浏览器回归；新增 `docs/s3-backup-operations.md`，同步主方案/实施文档/DESIGN/索引。保留原有 README 内容与工作区无关改动，未提交、推送或发布。

| 场景 | 实际证据 |
| --- | --- |
| PutFile、分片创建/分片/完成响应丢失 | `small_upload_unknown_success...`、`multipart_unknown_results...` 离线通过 |
| 源文件变化、过期上传和取消 | 源变化/expired/cancel 专项通过，旧空间取消清理不饿死当前空间 |
| record 发布失败保护旧转录 | 内容摘要路径回归通过，删除全部旧版本通过 |
| Range 不支持 | `restore_range_ignored...` 通过，偏移 `[0,4096,0]`，无错误拼接 |
| 远端版本变化 | `restore_changed_remote_version...` 通过，丢弃旧暂存后偏移 `[0,0]` |
| 实际关闭/重开 DB 续传 | `restore_resumes_partial_bytes...` 通过，偏移 `[0,4096]` |
| 删除意图重启、墓碑成功但清理失败 | 生命周期测试通过，刷新仍 pending_delete，重试精确清理 |
| 墓碑 412、写入成功但响应丢失 | `lifecycle_tombstone_conflict_and_unknown_success_reconcile` 通过 |
| 删除期间关闭设置/改只读 | 墓碑后冻结，0 后续 Delete/Abort，操作仍 blocked |
| 数据损坏、错误媒体头、同名文件 | 恢复测试通过，不提交损坏文件、不覆盖同名文件 |
| 首次摘要前本机文件替换 | 注册 Source 后替换文件，释放返回 BACKUP_LOCAL_CHANGED，文件保留 |
| 隐藏历史与本机释放 | 独立隐藏表/刷新不复现、读者释放后再恢复、零远端写通过 |
| 请求 429/5xx 退避与 403 阻断 | `retry_policy_persists_403_block_and_429_5xx_bounded_backoff` 通过：在调度失败持久化边界注入服务错误码，验证403立即阻断、429/5xx/412退避及第5次阻断；未模拟真实HTTP服务响应，真实403不计在线通过 |
| 真正磁盘耗尽、系统掉电、Windows 符号链接权限 | NOT_RUN；IO错误/路径校验分支不能替代这些系统级条件 |

增量 `cargo test --locked --manifest-path src-tauri/Cargo.toml backup -- --test-threads=1`：35通过/1ignored，迁移筛选1通过。完整迁移 `--test backup_migration`：6通过。HTTP body 计数专项通过，验证未改变传输正文。真实测试入口仍默认 ignored，已扩展为独立读者数据库、文案零视频读取、恢复/释放/删除/墓碑闭环，未用403环境冒充这些场景通过。

Windows 原生：独立 QA identifier `me.disign.yingqu.s3qa-cbdd3176ee03`，启动前新建空数据库并设置 workspace 内测试下载目录，避免迁移或读取原有用户数据。`npx tauri build --debug --features desktop --no-bundle --config .local/s3-native-qa.json` 退出0；实际 WebView/IPC 启动、任务/设置入口、无独立云端导航、远端备份表单与默认只读角色验证PASS。原生未填写真实凭据，未访问用户视频；仅终止本次QA进程，保留其隔离数据供追溯。此验收不覆盖原生视频解码、真实ASR模型、系统凭据弹窗、安装器及缩放125%/150%。

真实 R2：此前403阻塞已由用户修正配置，最新复验结果见下。早期403运行未能核验清理终态，不能用后续独立run的成功替代其历史状态。手动接管规程已落盘，旧凭据撤销/新主接管真实验证NOT_RUN。macOS Intel/Apple Silicon UNAVAILABLE；未运行三平台CI，不发布构建产物。

完整 M6 在线/跨平台验收尚未通过，不能据此宣称可发布版本。

最终独立 CHILD_AGENT 离线代码审查 PASSED，覆盖既有 M1–M5 分区、集成及最后 `record_failure` 提取与重试测试，无未解决 P0/P1/P2。最终增量目标 SHA-256：`scheduler.rs` 为 `25C919565ADBECD75A0E6EA769F8DDAC0462E95EA9814391281A1A1E3BFB8778`，`tests.rs` 为 `DF50B978557084A1ADE3F6876768A72AB9771775A170868EF6A5F339D00CFE4B`。此结论只覆盖代码及离线验证，不替代上述在线、原生媒体和跨平台验收。

### 2026-10-30 配置修正后的真实 R2 复验

授权：用户明确确认根目录配置已修正，只操作配置中的测试 Bucket 和前缀。未输出凭据，未访问用户视频，未修改 Bucket 或凭据权限。每次场景使用配置前缀下的新 UUID run 子前缀和独立临时数据库。

Files：本次只修改 `src-tauri/src/backup/tests.rs`、本验证记录、`docs/s3-backup-operations.md` 和主方案状态。生产代码未变；保留原有工作区改动，未提交、推送或发布。

实现：保留小文件测试入口，增加 `real_r2_m6_multipart_restart_dedicated_prefix`。测试专用 `ObservedR2` 在 ObjectStore 边界统计方法和成功分片调用；真实首分片成功后模拟响应丢失，关闭唯一 Core 后重开同一 SQLite，以真实 ListParts 对账。普通验收失败沿 Result 进入统一清理；清理仍限定本次资产的持久 owned keys 和 run 白名单对象。新增正式测试清理前的 HEAD 检查，避免测试清理掩盖产品删除失败。

| 实际命令 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml real_r2_m2_dedicated_prefix -- --ignored --nocapture` | 原有小文件闭环1通过，17.50秒，退出0；`.local/s3-m6-r2-retest.log` |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml real_r2 -- --ignored --nocapture --test-threads=1` | 最终冻结版本2通过，92.76秒，退出0；`.local/s3-m6-r2-final.log` |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | 退出0；`.local/s3-m6-r2-clippy.log` |

验证结果：

- 小文件与100 MiB生成夹具均完成上传、独立只读设备发现、文案读取、按需完整恢复及SHA-256核验、释放、再次恢复、主设备删除、墓碑刷新和拒绝再次恢复。
- 分片重启后保留 upload_id，真实 ListParts 识别首个已成功分片；应用层1～7号分片各成功调用一次，不重复传首片。该计数不宣称 SDK 内部 HTTP 请求没有重试，也不代表系统掉电测试。
- 两场景读者审计各为 GET=25、HEAD=2、LIST=2，无写入或 multipart 调用。记录/文案导入后本机视频路径仍为空，直到主动恢复。
- 产品删除完成后、测试清理前，媒体与转录 HEAD 均不存在；读者刷新得到 deleted，释放其副本后再次恢复被拒绝。
- 两场景均核验本次run对象列表为空、该资产拥有的媒体键下无未完成分片。仅清理本次测试数据，不清理其他前缀。

Review：独立 CHILD_AGENT 增量复审 PASSED，无未解决 P0/P1/P2。首审发现断言panic可能跳过清理，已改为可传播错误并复跑通过。最终 `tests.rs` SHA-256：`9859A4799E8CD22882554DE635D5EB27E5D2CC0A5642E43A2DBCC573AB5B3DAE`，替代上段旧测试文件目标。首次新增测试编译暴露 Fixture 所有权移动错误，修正后编译与上述运行通过，未修改生产行为。

当前结论：M6真实R2核心闭环及分片重启续传 **PASS**。完整M6仍保留以下缺口：管理侧旧写凭据撤销及主设备接管 **NOT_RUN**（现有S3对象凭据不能执行管理侧撤销）；macOS两架构 **UNAVAILABLE**；原生媒体解码/ASR、安装器、125%/150%缩放和上述系统级故障条件 **NOT_RUN**。这些缺口不由本次R2成功替代。

### 设置与卡片交互修正

用户要求：修复输入聚焦裁切，凭据默认掩码回填并提供显隐，未保存也能测试，测试中加载反馈、结果Toast；卡片模式同步图标移到封面左下角。

Files：`src/components/backup-settings.tsx`、`backup-tasks.tsx`、`src/main.tsx`、`src/style.css`、`src/api.ts`、`src-tauri/src/desktop.rs`、`backup/settings.rs`、`backup/tests.rs`、`tests/s3-backup-browser.mjs`，以及 DESIGN、主方案、实施文档、运维和本记录。

Contracts：滚动区为焦点外环留4px；专用凭据读取仅由设置编辑器使用，普通设置摘要仍脱敏，前端不持久化密钥。测试连接改为当前草稿的读取连接/库身份检查，不写配置、凭据、库、探针或备份队列；不将其解释为写权限测试。未修改凭据且目标未变化的保存继续省略密钥，保留离线关闭路径。保存恢复掩码，加载反馈复用Spinner和Toast。列表图标保留标题旁，本机和远端资产卡片均置于封面左下8px，无重复图标。

Tests：`npm run build`退出0（`.local/s3-settings-build.log`）；`cargo test --locked --manifest-path src-tauri/Cargo.toml draft_connection -- --test-threads=1` 2通过（`.local/s3-settings-rust.log`）；相同命令筛选 `credential_rotation` 1通过（`.local/s3-settings-credentials.log`）；desktop all-target Clippy退出0（`.local/s3-settings-clippy.log`）。首次新测试闭包缺少泛型类型标注导致编译失败，补充Memory类型后通过。

Browser：`playwright-cli --session s3-backup run-code --filename tests/s3-backup-browser.mjs` PASS（`.local/s3-settings-browser.log`）。1440×900、1280×800、1024×680的焦点留白、列表/卡片布局、凭据显隐、草稿测试无保存调用、按钮禁用/Spinner/aria-busy、成功及失败Toast、保存保留掩码均通过；截图 `.local/s3-settings-focus-fixed.png`、`.local/s3-feed-icons-fixed.png` 已查看。无pageerror。

本轮没有访问真实R2或读取真实凭据，未控制用户桌面、未重启当前原生窗口；浏览器mock不能替代原生系统凭据回填的验收。

Review：最终独立 CHILD_AGENT 增量审查 PASSED，无未解决P0/P1/P2，代码、测试和文档契约一致。Rust测试文件SHA-256为 `4EED385844151AF6238AB7601FF4CF92669BBF685982F00A15D07C8768A3BBDD`，浏览器测试为 `B3485C30F926A0C8848EB6FCBCBE059D9431C01B9760690A6906A67D7B708BE1`。`git diff --check`及受影响新文件空白检查通过。

### 卡片图标与桌面产物更新

用户仍看到标题旁旧云朵，是之前启动的exe未包含已完成的界面修正。本轮复用Lucide CloudUpload替换Cloud，20px、1.75px图标笔画，云轮廓使用原生SVG `fillOpacity=0.2`，按真实字节裁剪填充，完成仍是绿色对号。列表位置不变，卡片继续位于封面左下8px。

Files：`src/components/backup-tasks.tsx`、`tests/s3-backup-browser.mjs`、`DESIGN.md`及本记录。浏览器三视口回归PASS（`.local/s3-cloud-upload-browser.log`），覆盖新图标、本机/远端卡片位置和标题无重复图标；截图已查看。独立复审PASSED，组件SHA-256为 `BE8CA39DEC8FE655DB098D8A80272F285296A2515F0ABD1E2CCA33D199542C00`。

最终 `npm run desktop:build` 退出0（`.local/s3-cloud-upload-desktop-final.log`），包含前述设置修正及最新图标，生成 `src-tauri/target/release/yingqu.exe`；构建前引擎校验为0.14.0。当前运行旧exe仅在同一release目录改名备份，未终止进程或操作用户桌面。旧窗口需从托盘退出后打开新exe，不能把磁盘构建成功等同为已更新运行中的界面。未生成安装器，未提交或推送。

### 同步按钮直接操作与远端错误Toast

用户进一步要求缩小图标、背景同播放按钮、取消同步子菜单，以及远端错误只Toast。本轮图标改16px，卡片背景/悬停色复用播放按钮，完成态仍保持同色背景与绿色对号。待上传点击入队，上传中点击取消；待下载显示云下载图标，点击恢复，下载中再次点击取消。已备份且本机有副本的对号为禁用状态指示。远端错误、封面请求失败使用Toast，轮询同错误及同操作/错误/尝试次数去重；普通平台下载错误展示不变。

Files：`backup-tasks.tsx`、`src/main.tsx`、`src/style.css`、`src/api.ts`、`desktop.rs`、`backup/scheduler.rs`、`backup/tests.rs`、浏览器测试及 DESIGN/主方案/实施文档/本记录。新增 `start_asset_backup` 复用既有队列与worker，在Rust检查启用/主角色/活动资产/本机文件，pending/running点击幂等，失败或取消后重新排队，不直接创建第二条上传任务。

Tests：`cargo test --locked --manifest-path src-tauri/Cargo.toml manual_upload -- --test-threads=1` 1通过（`.local/s3-direct-actions-rust.log`）；desktop all-target Clippy退出0（`.local/s3-direct-actions-clippy.log`）。`playwright-cli --session s3-backup run-code --filename tests/s3-backup-browser.mjs` 最终PASS（`.local/s3-direct-actions-browser.log`），覆盖三视口、两类卡片位置、16px及播放同色、完成态不褪色、上传/取消/恢复直接操作、只读限制、没有子菜单或内联远端错误，以及Toast自动消失而不被轮询反复延长。

真实R2、本机原生交互未运行；没有读取真实凭据或操作用户桌面。原生旧进程保留，新产物需要退出旧进程后打开，未提交或推送。

最终 `npm run desktop:build` 退出0，见 `.local/s3-direct-actions-desktop-final.log`，新exe位于 `src-tauri/target/release/yingqu.exe`。独立 CHILD_AGENT 最终复审PASSED，无未解决P0/P1/P2；CSS SHA-256为 `A2168EC745F13721691EE00E894D95ECF4C7F041F04C5D6C2CC5C2D0FDA26262`，浏览器测试为 `0E45E27EFBFCC9DB45B778AF6543C7385B505F37D088920E255281A594B65F36`。差异空白检查通过。

### 保存反馈与页面稳定性

Files：`src/components/backup-settings.tsx`、`src/style.css`、`tests/s3-backup-browser.mjs`、`DESIGN.md`及本记录。保存按钮使用同格叠放的文字和Spinner，固定尺寸，250ms ease-out切换；快速请求至少展示300ms加载反馈。保存/测试期间继续锁定编辑，表单控件保持原有透明度，避免整页变淡闪烁。成功/失败均使用Toast。

Tests：`npm run build`通过（`.local/s3-save-feedback-build.log`）。`playwright-cli --session s3-backup run-code --filename tests/s3-backup-browser.mjs` PASS（`.local/s3-save-feedback-browser.log`）；1440×900、1280×800、1024×680下核验保存前、中、后的表单和按钮尺寸、位置、滚动位置、输入背景与透明度不变，表单DOM未重新挂载，保存中禁用/Spinner/aria-busy、失败Toast、250ms过渡及减少动态效果均通过。无pageerror。第一次布局断言受到Playwright自动滚动按钮的影响，改为点击前先滚动到按钮再比较；未弱化布局断言。

没有访问真实R2、读取真实凭据或操作用户桌面。原生视觉验收未运行，浏览器验证使用隔离mock。

最终 `npm run desktop:build` 退出0（`.local/s3-save-feedback-desktop.log`），构建前引擎0.14.0校验通过，生成 `src-tauri/target/release/yingqu.exe`。当前旧窗口未被终止，新界面需退出旧程序后打开新产物。未生成安装器、未提交或推送。

Delivery review：APPROVED；CHILD_AGENT / FULL_SCOPE；Review gate PASSED，Coverage COMPLETE，Freshness FRESH，repair rounds 0，无P0/P1/P2。最终实现目标为 settings `24AFBD1D509854BA9EEAD4F88DA4D18C77E2D965B9B344CDAA81A8FEC40CADE6`、CSS `0175B47E9646EF11D8CC0E6B5AD5150C70DD62E0284AD72FF50D9949EE5F5EC9`、browser test `718E30170D148FC053B91109154F6BDD23B0EA94127E2D8C0F9B9264175835E7`；交付时哈希一致。差异空白检查通过。

### 备份图标浅色背景与悬停反馈

Files：`src/style.css`、`tests/s3-backup-browser.mjs`、`DESIGN.md`及本记录。按最新要求替代之前与播放按钮同色的方案：封面左下备份按钮改为白色surface底、0.5px边框，保留深色轮廓、蓝色进度和绿色对号，对号笔画加粗。悬停/键盘焦点以250ms ease-out切换浅蓝背景并缩放至1.08；完成态通过外层容器响应悬停，仍禁用点击。减少动态效果时移除缩放/过渡。

`playwright-cli --session s3-backup run-code --filename tests/s3-backup-browser.mjs` PASS（`.local/s3-light-icons-browser.log`），覆盖三视口布局、两种视图、浅色底和绿色对号、不透明完成态、可操作及完成态的移入/移出效果、减少动态效果与原有直接操作行为。截图 `.local/s3-feed-icons-fixed.png` 已查看。未访问真实R2、读取真实凭据或操作用户桌面，原生交互验收 NOT_RUN。

最终 `npm run desktop:build` 退出0（`.local/s3-light-icons-desktop.log`），包含前端类型/构建检查和0.14.0打包引擎校验，生成 `src-tauri/target/release/yingqu.exe`。未终止原有窗口，需退出旧程序后打开新产物。

Delivery review：APPROVED；CHILD_AGENT / FULL_SCOPE；gate PASSED、Coverage COMPLETE、Freshness FRESH，repair rounds 0，无P0/P1/P2。最终CSS SHA-256 `DEC835609E9B1904728C4F8A7595529E2EB755D4AE4B52EF2325B5146A730B4B`，browser test `91C14A3C9FB0FF30AC20706A3E700A178839FD64D388AA134165CED48C892838`；交付时与审查目标一致，差异空白检查通过。

### 备份图标灰底无边框

按最新要求将封面左下备份按钮从白底改为 `--surface-control`（Radix slate-3）浅灰底，移除边框；悬停使用更深的 `--surface-hover`（slate-4），保留原有缩放、语义图标颜色和减少动态效果。仅修改 `src/style.css` 两条规则、已有浏览器测试的颜色/无边框断言以及 DESIGN 对应说明。

浏览器三视口回归 PASS（`.local/s3-gray-icons-browser.log`），无边框、常态/悬停颜色及原有交互通过，截图已查看。未访问真实R2、读取凭据或控制桌面；原生视觉验收 NOT_RUN。

最终桌面构建 `npm run desktop:build` PASS（`.local/s3-gray-icons-desktop.log`），已包含前端构建及引擎校验，产物 `src-tauri/target/release/yingqu.exe`。独立增量审查 APPROVED（CHILD_AGENT、FULL_SCOPE、gate PASSED、Coverage COMPLETE、Freshness FRESH，无阻断发现，repair rounds 0）。CSS `32A3AE74C91F2BD3B06C079024F7640DB3AA5CFF28A1B1C511420FB39B69D132`、测试 `4547735263B3175E25A8307358D40CC5F8DA192E25C433F68675565B9A2B47F8` 与审查状态一致。差异空白检查通过。旧窗口未重启，需要退出旧程序后打开新产物。

### 标题前的小尺寸备份图标

用户将位置调整为标题前方，并要求与文字匹配、无背景。修改 `src/main.tsx`、`src/components/backup-tasks.tsx`：本机任务和远端资产在列表/卡片中均只在标题前渲染一次备份状态，移除封面上的图标。`src/style.css` 使用普通行内flex布局和4px间距，卡片图标与标题首行对齐；图标14px，点击区域20px，透明无边框，保留悬停缩放及减少动态效果，操作与权限不变。DESIGN 和既有浏览器断言同步更新。

浏览器三视口回归 PASS（`.local/s3-title-icons-browser.log`），覆盖标题前位置/首行对齐、封面无重复图标、14px尺寸、透明无边框及原有悬停/直接操作/权限/设置行为。截图已查看。未访问真实R2或凭据，未控制用户桌面，原生视觉验收 NOT_RUN。

最终 `npm run desktop:build` PASS（`.local/s3-title-icons-desktop-final.log`），含前端类型/构建及打包引擎校验，新产物 `src-tauri/target/release/yingqu.exe`。独立审查 APPROVED（CHILD_AGENT/FULL_SCOPE，gate PASSED、Coverage COMPLETE、Freshness FRESH，repair rounds 0，无已确认缺陷）。最终hash：main `DC2A018E8E51CA0AB78DB848148DB19DB93A01447DF31D68A7AC5F6D1942820D`、backup-tasks `97DFEAB2D342207CE6AC7791DF595850D27489D4B20861B66B6A566DEE25B025`、CSS `A6A3C2C5F132FC9E1F8E3293E7952AE1A3F19BE4CA58268C17983ECCEC16A1CB`、测试 `E04F4B5D2E48CC7C606456F205C3E690E2227ACAAC0FF4E5A207F648200572C6`，交付时与审查目标一致。差异空白检查通过。旧窗口未重启。

### 圆圈状态图标

根据用户参考图，将 `SyncIcon` 统一为Lucide CircleCheck、CircleArrowUp、CircleArrowDown，14px、1.75px笔画，保留标题前位置及透明无边框；完成态绿色圆圈对号，上传/恢复圆圈箭头继续按实际字节自下向上填充。删除完成态单独的2px笔画覆盖，未修改任何操作逻辑。

Files：`src/components/backup-tasks.tsx`、`src/style.css`、`tests/s3-backup-browser.mjs`、`DESIGN.md`及本记录。浏览器三视口回归 PASS（`.local/s3-circle-icons-browser.log`），验证三类圆圈图标、上传50%/恢复75%的裁剪填充、原有尺寸/布局/悬停/直接操作，截图已查看。初次布局读取跨越响应式重排导致两次boundingBox来自不同帧，改为同一次evaluate读取标题和图标的矩形，保持原布局断言。没有访问真实R2或凭据，未控制用户桌面；原生视觉验收 NOT_RUN。

最终 `npm run desktop:build` PASS（`.local/s3-circle-icons-desktop.log`），包含前端构建和打包引擎校验，已生成新 `src-tauri/target/release/yingqu.exe`。独立增量审查 APPROVED（CHILD_AGENT/FULL_SCOPE，gate PASSED、Coverage COMPLETE、Freshness FRESH，repair rounds 0，无已确认缺陷）。最终组件 `E925EC6F41F435044E12E7FE46FBFF78F70749166BA0F13335BCF340DBB2896F`、CSS `A26FD78B284D8E784144A3B20BDE574F7F19BA8B218814F135210E8F202C7ADC`、测试 `7ED3C45BB6F0519517AEF76BB5BACD10B1CEE2DBDE2E62590170BD28EF1A0CE4` 与审查状态一致。差异空白检查通过，旧窗口未重启。

### 启动不重复弹出历史备份错误

只读检查当前应用数据库的备份状态聚合，发现5条上传在尝试5次后停在blocked，历史错误为BACKUP_SERVICE_UNAVAILABLE，另有154条完成。未读取凭据或修改数据库，未发起远端请求。前端原来在首次加载持久化操作时将所有last_error当成新错误弹Toast。

修改 `src/components/backup-tasks.tsx`：在首次成功加载备份空间时建立操作错误基线，后续错误身份变化仍正常Toast；保留当前刷新/直接操作的失败提示和历史任务图标悬停原因。基线按space_id区分，避免把另一个空间的历史记录当成新事件。未改变队列、重试或远端访问行为。

`tests/s3-backup-browser.mjs` 使用blocked/attempt=5的历史错误fixture，启动多轮加载不弹该Toast但悬停保留原因；现有后续新错误、当前刷新失败和去重测试继续通过。三视口浏览器回归PASS（`.local/s3-startup-toast-browser.log`）。DESIGN同步通知规则。没有访问真实R2；本次不证明远端当前可用，也未重试那5条真实失败任务。原生交互验收 NOT_RUN。

最终 `npm run desktop:build` PASS（`.local/s3-startup-toast-desktop.log`），包含前端构建和引擎校验，已生成 `src-tauri/target/release/yingqu.exe`。独立审查 APPROVED（CHILD_AGENT/FULL_SCOPE；gate PASSED、Coverage COMPLETE、Freshness FRESH；repair rounds 0，无已确认缺陷），组件 `D82E353659747EA61896D0B23FEC7114CE8D0D1095EE0E44F22ED3A5247C9A1F`、测试 `E470CD05C9961D4BD2F6101B0BADE3438862100898846BCC059023902D309B04` 与最终状态一致。差异空白检查通过，当前旧窗口未重启。

### 图标原地缩放与右键传输操作

用户要求已完成图标悬停不横移，右键新增上传/远端下载。`src/style.css`固定20px按钮的位置和尺寸，仅内部14px `.backup-sync-glyph` 以自身中心缩放1.08，减少动态效果时不缩放。`src/components/backup-tasks.tsx`增加共享传输菜单项，本机任务经`TaskMoreMenu`插槽接入，远端资产的右键和更多菜单复用相同内容；主设备有文件/尚未备份时可上传，远端已备份/本机无文件时可恢复，运行中禁重复，读者禁上传。复用原命令与Toast，未新增后端操作。远端行加入现有右键允许区域并支持Shift+F10。同步DESIGN。

Files：`src/components/backup-tasks.tsx`、`src/components/task-more-menu.tsx`、`src/main.tsx`、`src/style.css`、`tests/s3-backup-browser.mjs`及设计/本记录。

`npm run build` PASS（`.local/s3-transfer-menu-build.log`）；浏览器回归PASS（`.local/s3-transfer-menu-browser.log`），覆盖本机右键上传命令、远端键盘菜单恢复、传输中/已有本机副本/完成态/只读角色限制、列表和卡片菜单，以及悬停300ms逐帧图标中心不变、按钮矩形不变、减少动态效果与既有三视口回归。菜单首次测试发现远端行未列入全局右键允许区域，已加入现有task-context-target；动画测试点击菜单后的滚动会影响坐标，改为测量前先滚入可视区域，保留原位置断言。

未访问真实R2或凭据、未操作用户桌面；原生视觉验收NOT_RUN。

最终 `npm run desktop:build` PASS（`.local/s3-transfer-menu-desktop.log`），包含前端构建及打包引擎校验，生成 `src-tauri/target/release/yingqu.exe`。补充浏览器检查远端行Shift+F10打开、Escape关闭后焦点回到原行。独立审查 APPROVED（CHILD_AGENT/FULL_SCOPE，gate PASSED、Coverage COMPLETE、Freshness FRESH，repair rounds 0，无P0/P1/P2），最终组件 `3A5DB4C02DD9D08D892A198C5231B7D851659B57391E68314DEDED8FDCFE95EF`、task-more-menu `F5BF98BB5DC26AB43BDA30479509C1604DF515B53A2914CECC7A3708C8DB67CD`、main `099CDD241C52E1AE54DDF5029C919D69A4EFFA76E1EC14F56BCA4B6112A45C51`、CSS `8B05CE10003BCEB4B78339B508EBB700E9DA6CF7ADFDC630C0D1EEB2DA36E999`、测试 `DF7A3EE26475E4E368AF2321D03B14247FD20692372E75C9711CD4C4CD5B82D0` 与审查状态一致。差异空白检查通过，旧窗口未重启。

### 单行标题紧凑排布

删除 `src/style.css` 信息流标题的 `min-height:40px`，标题按实际一行/两行占高，保留两行截断和6px信息间距；单行下方作者/来源不再被空白第二行推开。DESIGN对应布局说明同步。

既有 `tests/s3-backup-browser.mjs` 增加三视口单行标题20px高度、信息间距6px断言，并验证长标题仍为40px/两行截断。浏览器回归PASS（`.local/s3-compact-title-browser.log`），截图已查看。无后端、远端或凭据操作，未控制用户桌面；原生视觉验收NOT_RUN。

最终 `npm run desktop:build` PASS（`.local/s3-compact-title-desktop.log`），包含前端类型/构建与引擎校验，生成 `src-tauri/target/release/yingqu.exe`。独立审查 APPROVED（CHILD_AGENT/FULL_SCOPE；gate PASSED、Coverage COMPLETE、Freshness FRESH；repair rounds 0，无已确认缺陷）。CSS `53530E086FFB17B6AE889B4B0D96BD7B25680085DBE7E3D415117E3093248932`、测试 `B88D0E1B6C23B11F8E47D8EF773EDB0E34C9AAD8CCCA6C2E1DAC389A6FB8F83E` 与最终状态一致。差异空白检查通过，旧窗口未重启。

### 待上传云朵与上传圆环反馈

`src/components/backup-tasks.tsx` 将待上传图标改为空心Cloud，点击后立即用现有Spinner反馈请求，记录本次点击意图使入队后等待阶段持续反馈，并支持取消排队上传；有实际字节进度时复用现有圆形Progress按比例顺时针补满。最多99%，远端记录提交成功后才切绿色CircleCheck。保持14px/透明背景/原地悬停；S3恢复图标和后端协议不变。`src/style.css`覆盖小圆环尺寸与透明底，提交期间不整体变淡。DESIGN同步状态说明。

`npm run build` PASS（`.local/s3-upload-ring-build.log`）；浏览器回归PASS（`.local/s3-upload-ring-browser.log`），覆盖空心云朵、延迟上传请求期间立即Spinner/禁重复且无虚构百分比、入队持续反馈、取消恢复云朵、50%/75%圆环角度、传完字节保持99%及记录提交后圆圈对号，以及原有三视口/菜单/权限/恢复回归。截图已查看。未访问真实R2或凭据，未控制桌面；原生视觉验收NOT_RUN。

最终 `npm run desktop:build` PASS（`.local/s3-upload-ring-desktop.log`），包含前端构建和引擎校验，已生成 `src-tauri/target/release/yingqu.exe`。独立审查 APPROVED（CHILD_AGENT/FULL_SCOPE，gate PASSED、Coverage COMPLETE、Freshness FRESH，repair rounds 0，无已确认缺陷）。组件 `616D78B2131AB8736A2457241A0504077AC3DCB9CA2FA0EDBA36D54167B4DE2D`、CSS `985304CBD221193CC536CDD84E1B5A1224AAC8A6B81DF5BE577F99C3186864DD`、测试 `6F07CB3171544788ECEC025BF1F92007F4E3A60C41A97D5E7595CA3C8E65F4E7` 与审查状态一致。差异空白检查通过，旧窗口未重启。

### 标题与备份图标固定单行

按用户最新要求，删除卡片标题的两行line-clamp和换行覆盖，复用列表单行nowrap/ellipsis，补充min-width:0确保长标题可收缩；删除首行顶部对齐覆盖，图标与单行标题中心对齐。远端资产补充原生title以查看完整标题，本机任务已有该属性。同步DESIGN，替代此前保留两行的约定。

Files：`src/style.css`、`src/components/backup-tasks.tsx`、`tests/s3-backup-browser.mjs`及设计/本记录。浏览器回归PASS（`.local/s3-single-line-title-browser.log`），三个视口分别将本机/远端标题设为超长中文带标签文本，验证标题及heading均20px、真实溢出且ellipsis、图标中心不偏移；保留其余备份操作回归。未访问真实R2/凭据或操作桌面，原生视觉验收NOT_RUN。

补充使用用户截图中的长标题做隔离浏览器截图 `.local/s3-single-line-title.png`，已查看单行省略效果。最终 `npm run desktop:build` PASS（`.local/s3-single-line-title-desktop.log`），含前端构建与引擎校验，生成 `src-tauri/target/release/yingqu.exe`。独立审查 APPROVED（CHILD_AGENT/FULL_SCOPE，gate PASSED、Coverage COMPLETE、Freshness FRESH；repair rounds 0，无已确认缺陷）。CSS `F04EBBB249C3505C295740C903A5574ECBFD065754A6F1F237A33C46A17E6F4C`、组件 `A8DE344B73C91411718461AEA165E069EA89135252EE4B618F29B40FD9282E3A`、测试 `9D59FE2BC26D25D6CDA2BBC97D97639B14A4DC2337367A9FBD1AC9E73DD8ACAA` 与审查状态一致。差异空白检查通过；旧窗口未重启。

### 标题、备份图标和作者统一Tooltip

复用现有Radix/shadcn Tooltip，替换本机/远端任务标题、备份图标、AuthorInfo名称及ID的原生title；远端作者文本也使用相同提示。标题/普通作者文本可键盘聚焦，作者按钮保留原点击行为；备份图标使用inline-flex外层触发器，禁用/完成状态仍可悬停或聚焦，保留状态、操作及历史错误文本，按钮权限不变。

Files：`src/main.tsx`、`src/components/backup-tasks.tsx`、`src/components/author-info.tsx`、`tests/s3-backup-browser.mjs`、DESIGN及本记录。`npm run build` PASS（`.local/s3-tooltips-build.log`），浏览器回归PASS（`.local/s3-tooltips-browser.log`），验证本机/远端标题全文的hover/focus、两类作者hover、禁用已备份图标hover/focus、历史错误Tooltip及这些入口无原生title，原有三视口/上传/恢复/菜单/权限回归通过。未访问R2/凭据或操作桌面；原生交互验收NOT_RUN。

审查修复：初审指出合法超长标题/作者可使Tooltip高度超出窗口（P2）。本轮文本浮层增加Radix available-width/height约束、8px碰撞留白及overflow-y:auto，不改全局Tooltip或无关原生title。测试使用4500字符标题和2000字符作者，验证hover/focus后边界仍在视口内，超长内容可滚动，最终浏览器回归PASS。此前短文本边界检查不能替代此项。

最终 `npm run desktop:build` PASS（`.local/s3-tooltips-desktop-final.log`），包含前端构建与引擎校验，生成 `src-tauri/target/release/yingqu.exe`。独立复审 APPROVED（CHILD_AGENT/FULL_SCOPE，gate PASSED、Coverage COMPLETE、Freshness FRESH，repair rounds 1），长文本溢出P2已关闭，无剩余发现。最终main `BFDA27D862C6CE876CD9086A0B19F63F3B14ED1B8BB8712C137E6DF77AC64D10`、backup `7DC10E4D6D484550DF8C13541A18444A94DAF5C8ECF258BB5CC70EF94E464A7A`、author `E999C05218F4B3989287B345F86DF324C4C9417C3C52BF081ED8E33F32784CAC`、测试 `89664B9728C17A5B0DEB215766E509E144E0C2258F30F086D847ECF8ACEE45F1` 与审查状态一致。差异空白检查通过；用户并行修改的AGENTS规则保留，旧窗口未重启。

### 全局Tooltip浅黑背景

`src/style.css`的公共Tooltip规则统一使用foreground（Radix slate-12）浅黑实底、on-primary浅色文字，边界及箭头与背景同色；保留尺寸、布局和窗口避让。DESIGN的Tooltip规范同步。现有浏览器边界检查同时验证标题、作者和备份状态Tooltip的背景/文字/箭头颜色，回归结果见 `.local/s3-dark-tooltips-browser.log`。未访问R2或凭据，未控制桌面。

浏览器回归PASS，截图 `.local/s3-dark-tooltip.png` 已查看。最终 `npm run desktop:build` PASS（`.local/s3-dark-tooltips-desktop.log`），包含前端构建与引擎校验，生成 `src-tauri/target/release/yingqu.exe`。独立审查 APPROVED（CHILD_AGENT/FULL_SCOPE；gate PASSED、Coverage COMPLETE、Freshness FRESH；repair rounds 0，无新增缺陷）。CSS `28FEE7717DEC5D81054B5C6333082DDCCCD71F52C0E4B7045E53AB2D02974EDE`、测试 `C161F2B48E3D522942FD33D94817C259D3383AC46EE14A55D2F0C396A98BBE7B` 与最终状态一致。差异空白检查通过，旧窗口未重启，原生视觉验收NOT_RUN。
