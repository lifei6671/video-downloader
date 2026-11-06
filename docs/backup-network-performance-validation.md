# 备份弱网与资产列表性能验证

日期：2026-11-06。上一轮修复已交付于 `3bd6112cfe897577856f6bea4eafe18766548a39`。本轮以该提交和干净工作区为基线，补齐上传成功参与并发恢复的遗漏；未提交或推送。下述实现描述为当前状态，历史测量和验证单独标明。

## 实现与边界

1. **S3 RequestTimeout**：先按服务端错误码分类，HTTP 400 的 `RequestTimeout` 进入 `BACKUP_NETWORK_ERROR`，普通 400 拒绝不重试。分片沿用最多三次、1/2 秒退避与取消检查，每次重用 upload ID、part number 和字节范围；Create/Complete、条件写不进入局部重试。
2. **S3 响应期限**：保留 `60s + bytes / 64KiB/s` 总预算、15 秒连接期限和 20 秒停滞宽限。HTTP 客户端消费最后一帧后，再限制等待响应最多 60 秒，覆盖文件 PUT 与 UploadPart，且不依赖进度回调。这个起点包含刷新最后的 socket 缓冲，不代表远端已确认接收。
3. **Telegram 慢速分片**：sender 从固定 24 秒改为 60 秒无 socket 进展、599 秒总期限；分片和媒体恢复探测外层均为 600 秒。上传/下载文件 Io 使用 2,410 秒活动等待预算，容纳最多四次分片尝试与退避；元数据操作仍为 60 秒。未取消的请求到期仍淘汰对应连接，取消不参与健康超时。加密响应解析前不能归属单个请求，因此连接读取会刷新该连接的文件请求；总期限仍限制持续碎片流量。
4. **Telegram 账号并发与授权**：请求取得许可时记录预算代次，超时仅在代次仍匹配时降一档并推进代次，同批错误广播不会连续降至 1。当前代次至少 32 次连续文件成功且稳定至少 30 秒后升一档，最多回到 16；RPC 返回本次请求的预算代次，由调用方在有效确认后计数，大/小上传的 true 和校验通过的下载分片均可推动恢复。Route 仅保留迁移/认证错误语义，控制请求成功不计数，上传 false 及无效下载数据按失败清空当前代次成功记录；失败清空成功计数并重启稳定期，旧代次结果不影响新窗口。已排队请求复查代次，预算由同账号的后续文件继续复用。授权成功代次移出账号预算，改由实际 VaultSession 按 DC 持有；只有同会话同 DC 的失败等待同一导入锁，正常分片不等待网络授权。下载和备份从其实际会话取得同一状态，独立登录同账号时各自导入；重新加载会话重建该状态。
5. **首次备份准备**：历史定位、归属与旧附件校验移到大文件上传前。视频确认后，封面和文案从同操作、同设备/代次的确认日志取得视频消息 ID，即使资产投影尚未发布也不再各扫一次历史。没有可靠位置或发送结果未知时继续历史对账；`Sending` 仍紧邻实际发布 RPC，旧未知发送记录不会自动重置。
6. **资产列表**：Tauri 命令移至 `spawn_blocking`，通过独立只读连接的单事务批量读取。完整显示路径固定五条 SELECT，不按资产或空间追加独立查询；其他空间只读备份位置字段，当前空间才解析完整远端快照。保留隐藏、删除、冲突及附件语义。文件存在性按路径缓存 30 秒，轮询使用缓存；显式刷新与操作后的读取强制核验，省略新增 IPC 参数仍保持立即检查。
7. **操作列表**：按是否提供空间拆开查询，按空间读取可使用既有索引，保留原排序与完整轻量历史。

上传前、流式上传中、上传后的完整摘要核验暂时保留。评审将其列为低优先级优化；当前没有引入稳定文件身份契约，不以大小/mtime 替代完整核验。文件缓存仅用于列表展示，不替代上传、恢复或删除的真实校验。S3 客户端复用、整任务断点续传和多设备同时写入也未扩展。

## 历史本地性能测量（上一轮）

Windows debug Rust，真实临时 SQLite/WAL，501 条资产，其中 500 条同时属于两个空间。单次完整显示读取为 **12.512 ms**，缓存路径为 **10.269 ms**，包含 Rust 结果构造；不包含 IPC 与界面渲染，不代表线上 P95。测试同时验证主任务锁被另一个线程持有时，显示读取仍能在两秒测试期限内完成。

上轮 2026-11-05 的操作投影测量是历史证据，本轮没有重测：5,000 条、每条约 128 KiB payload 的查询从 1,149.092 ms 降至 19.858 ms。不能将该数字与本次资产列表混作同一场景。

## 本轮验证

回归直接调用 `TelegramSink::save()` 和假 `PartClient`，分别覆盖 SaveFilePart/SaveBigFilePart：第一次 TimedOut 后同分片重试，预算从 16 降为 8；保持账号预算并切换 file_id，31 次有效确认加控制请求仍不恢复，第 32 次有效上传确认在稳定 30 秒后恢复至 16。小文件使用两个最多 16 片的文件覆盖跨文件门槛。

另验证服务端 false 清空成功计数且不恢复、请求在旧代次发出而在新代次完成时不推动恢复，以及有效下载仍可恢复但短片/CDN 响应不计成功。所有请求使用合成客户端；未访问真实 Telegram。

| 检查 | 结果 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib --locked telegram_transfer_engine -- --test-threads=4` | PASS，33 passed |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked -- --test-threads=4` | PASS，核心库 505 passed / 27 ignored，集成 18 passed / 3 ignored，doc-tests 0 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets --locked -- -D warnings` | PASS |
| `git diff --check` | PASS |

本轮新增四项回归，上述引擎测试也包含在完整测试中，不能重复累加通过数。独立 CHILD_AGENT FULL_SCOPE 复核覆盖三个代码文件、两份文档，以及上传入口、下载和控制 RPC 的全部调用关系；最终以冻结目标的交付记录为准。真实 Telegram、原生 Tauri、macOS 和安装包验收为 **NOT_RUN**，未使用真实凭据。前端和 vendor 源码没有改变，本轮未重复其独立测试。

## 上一轮验证（3bd6112）

针对超时广播、独立故障降档、稳定恢复和保留预算的后续文件，执行真实引擎重试/许可逻辑配合合成 RPC。独立会话测试使用两个各自持有 DC 授权状态的假客户端，每个会话两个文档、共八个并发分片；两边必须各导入一次并完整返回数据。慢 DC 导入使用有界 gate 阻塞，同时断言另一个已授权 DC 可完成。不是在线 Telegram 或真实 sender 广播验收。

| 检查 | 结果 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked -- --test-threads=4` | PASS；核心库 501 passed / 27 ignored，集成 18 passed / 3 ignored，doc-tests 0 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets --locked -- -D warnings` | PASS |
| `git diff --check` | PASS |

29 项传输引擎测试包含在本轮完整测试中。新增四项回归验证广播降档/恢复、双会话并发导入、慢 DC 隔离，以及成功次数与稳定期限必须同时满足、失败重置计数。旧代次超时在并发档位恢复相同数值后仍不会再次降档。现有同会话八分片合并、主 DC 认证失效、导入失败及导入后仍未授权的终止边界保持。

真实 Telegram、原生 Tauri、macOS、安装包验收：**NOT_RUN**；未读取真实凭据。前端及 vendor 源码未变，本轮不重复其构建/浏览器/vendor 独立测试；下表保留此前证据，不作为本轮重新执行的结果。独立复核采用 CHILD_AGENT FULL_SCOPE，覆盖本轮六个代码文件、两份文档和会话上下游；最终状态以冻结目标的交付记录为准。

## 上一轮验证（bfc698d）

| 检查 | 结果 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked -- --test-threads=4` | PASS；核心库 497 passed / 27 ignored，集成 18 passed / 3 ignored，doc-tests 0 |
| `cargo test --manifest-path vendor/grammers-mtsender/Cargo.toml --lib --features proxy --locked` | PASS，21 passed；集成审查补强取消期限测试后定向重跑该项 PASS |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets --locked -- -D warnings` | PASS |
| `cargo clippy --manifest-path vendor/grammers-mtsender/Cargo.toml --lib --features proxy --locked -- -D warnings` | PASS |
| `npm run build` | PASS，TypeScript 与生产前端构建 |
| `tests/backup-delete-flow-browser.mjs` | PASS，三视口、列表/信息流、任务/纯资产共 12 场景，合成 IPC |
| `tests/telegram-backup-locations-browser.mjs` | PASS，任务/纯资产各三视口，位置 Tooltip、键盘、轮询缓存与操作刷新 |
| `git diff --check` | PASS |

新增回归包括：真实 SDK 接收 XML HTTP 400 RequestTimeout 后同会话重试成功，普通 InvalidRequest 400 不重试；128 MiB 请求体全部读完后服务端持续不回复，PUT/UploadPart 均在独立响应期限内结束；512 KiB 加密 Telegram 响应以 16 KiB/s 持续到达超过旧期限仍完整成功；16 个账号许可已占用并有等待者时降低预算；8 个并发跨 DC 未授权请求只导入一次；准备失败零上传、首次附件在无投影时复用视频确认日志；多空间批量读取、主锁隔离、文件缓存强制刷新及过期。

浏览器使用现有 Playwright CLI 和 Vite 开发服务，不访问真实平台。位置测试同步适配当前卡片的状态文本/错误文本 Tooltip，保留位置完整文案、键盘和窗口边界断言，并新增轮询缓存与操作刷新 IPC 检查。

真实 Telegram/S3/R2、原生 Tauri 运行、macOS、安装包验收：**NOT_RUN**。未使用真实用户凭据；忽略项不计通过。构建仍有既有 sourcemap/bundle size 提示和 Windows linker 提示。vendor 单独测试生成的临时 Cargo.lock 不纳入改动。

## 上一轮交付复核（bfc698d）

按 S3、Telegram、资产列表/数据库三个分区执行 CHILD_AGENT 独立只读审查，并以各文件 SHA256 冻结检查。三个代码 checkpoint 均 PASSED；S3 测试缺少有界等待的发现已修复并复核。集成审查另发现取消测试仍只推进旧 25 秒，已改为 40 秒后发起存活请求、再推进 21 秒越过旧请求的 60 秒期限，确保不混入正常媒体空闲回收；定向测试通过。文档、浏览器测试及跨分区契约在最终集成审查中核对，最终结论以冻结目标与实际验证证据为准。
