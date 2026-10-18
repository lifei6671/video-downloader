# M2 第三张卡：重试与历史管理验收

日期：2026-10-13。范围仅重试、历史移除、删除本地文件；沿用 yt-dlp 与现有签名补丁、默认 2 / 可设 1–3 队列。

## 行为

- 失败/取消且有作品身份：按 2026-10-18 用户最新要求，重试复用原任务 ID 和创建时间，使用最新配置重新解析，自动进入 FIFO 队尾，从头下载；清空上次进度与错误，列表不新增记录。以下 2026-10-13 验证表为历史证据，当时采用新建任务，不能代替此次原记录重试验收。
- 无作品身份：提供重新解析入口，用户重新粘贴链接。不存储原始分享文本。
- 从列表移除：确认后仅删终态记录与提交证据，不删本地视频；仍在执行或清理的任务拒绝操作。
- 删除本地视频：独立确认框展示路径，明确永久删除；返回/ Escape 不执行。成功保留历史，显示文件已移动或删除；文件操作失败保留记录并显示错误。
- 通过 task_removed 事件与前端已移除 ID 集合防止迟到更新/旧快照复活。

## 验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | PASS：18 项，新增重试配置、终态约束、删除事务失败、删除目录失败、移除保留文件、删除保留历史及重启检测 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASS |
| `cargo build --manifest-path src-tauri/Cargo.toml --features desktop` | PASS |
| `npm run build` | PASS；保留现有第三方 use client 打包提示 |
| Windows 原生 Tauri + 打包引擎 | PASS：TikTok 解析后取消，从 UI 重试得到 687,985 字节文件；仅删除本卡新下载测试文件 |
| 原生确认/历史操作 | PASS：返回不删除，确认删除保留缺失记录，移除后刷新不恢复 |
| 键盘 | PASS：更多按钮聚焦→ArrowDown→菜单确认框→Escape 回到触发按钮；成功移除后焦点回到当前导航 |
| 视口 | PASS：1440×900 原生；1280×800、1024×680 WebView 模拟下确认框完整、无水平溢出；验后重启恢复原生视口 |
| 旧快照过滤 | PASS：高 revision 的已移除任务也不会合并回来 |
| 抖音本卡在线重试 | NOT_RUN；本卡未改平台引擎，前一卡网络失败记录仍保留，不计为本卡通过 |
| 全新无 Python Windows / macOS / 安装包 | NOT_RUN：留在 M3 |

本地证据（忽略目录）：`.local/m2-history-native-results.json`、`.local/m2-history-ui-results.json`、`.local/m2-history-delete-confirm.png`、`.local/m2-history-1280.png`、`.local/m2-history-1024.png`。独立审查冻结范围见 `.local/m2-history-review-target.json`。

## 2026-10-18：在原任务上重试

用户覆盖原有“重试新建任务”约定。原 ID、创建时间、resolver 和规格保留，revision 单调递增；清空进度、错误与完成状态，以最新配置重新解析并排到 FIFO 队尾。重新解析失败仍保留原作品身份，允许再次重试；没有作品身份的旧解析失败仍需重输链接。旧 worker 资源未释放时拒绝重试，旧暂存按既有路径规则清理；旧提交意图与重试快照原子替换。最终视频或图集以本次下载 UUID 独立命名，不覆盖旧尝试已落盘的文件。

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib desktop_core::tests -- --test-threads=1` | PASS：29 项；含原 ID、最新配置、连续失败和再解析失败、规格保留、FIFO、重复提交、事务回滚、暂存清理、不覆盖旧文件及重启单记录 |
| `node --test tests/task-state.test.mjs` | PASS：原行更新、列表位置保留、迟到旧快照不覆盖重试结果 |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASS |
| `npm run build` | PASS；保留既有 use client / sourcemap 和体积提示 |
| Edge 浏览器 + 隔离 IPC mock | PASS：后台提交后失败，键盘 Enter 重试使用原 ID，原行清空错误并更新解析/排队/完成状态，首页可再次查看该作品；1440×900、1280×800、1024×680 无水平溢出 |
| 本次原生 Tauri 按钮与平台在线下载 / macOS 实机 | NOT_RUN；Rust 测试使用真实 Python 子进程和离线协议 fixture，浏览器 mock 不代表安装包或在线验收 |

界面证据：`.local/output/playwright/retry-failed.png`、`retry-queued-1440.png`、`retry-queued-1280.png`、`retry-queued-1024.png`、`retry-completed.png`。
