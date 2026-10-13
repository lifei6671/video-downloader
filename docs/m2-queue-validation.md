# M2 第二张卡：并发下载与 FIFO 等待队列

日期：2026-10-13。本卡只实现并发与等待队列，保留已有紧凑亮色 shadcn/ui 界面、yt-dlp 与抖音详情补丁。

## 行为

- 默认同时下载 2 个，设置页可选择 1–3 个并保存 SQLite。旧数据库自动补充该配置，默认值为 2，原下载目录不变。
- 每次提交单个作品；解析最多同时一个，可与下载并行。点击下载后按提交顺序进入 FIFO，获得名额才启动独占引擎进程重新提取并核对作品。
- 提高上限立即补位；降低上限不打断正在下载的任务，实际占用降至新上限以下才继续调度。
- 等待任务可取消且不启动引擎；取消运行任务等待进程退出与清理后释放名额。一个任务失败或取消不影响其他任务。
- 每个任务保留创建时的配置和目录快照。退出先禁止调度，再取消并等待所有操作；重启遗留 parsing/ready/queued/downloading 标为 INTERRUPTED，不自动恢复。
- 任务页显示下载数、等待数、并发上限与等待序号；侧栏计数来自真实任务。未增加重试、删除、暂停或断点续传。

## 验证

| 项目 | 结果 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | PASS，15 项；含 5 项新增队列/限额/迁移测试 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASS |
| `cargo build --manifest-path src-tauri/Cargo.toml --features desktop --bin yingqu` | PASS，Windows 原生开发程序 |
| `npm run build` | PASS，类型与前端构建 |
| 真实子进程 fixture | PASS：2 个槽位、FIFO、排队取消、下载失败补位、取消互不影响、1→3→1、非法 0/4 拒绝、配置快照、停止调度后退出清理、重启等待状态中断 |
| 数据库升级 | PASS：旧两列 app_settings 增加 max_downloads，默认 2，保留目录；改目录不重置限额 |
| 原生 UI / IPC | PASS：旧数据库显示默认 2；四任务提交后实际 2 downloading + 2 queued；切换 3 后实际达到 3 并行；降至 1 保留已运行的 3 个，之后按新上限补位 |
| 原生重启 | PASS：保存为 1，正常退出再启动仍为 1；验收结束恢复为 2 |
| 实际视口 | 原生 1440×900，无横向溢出；本轮未设置浏览器模拟视口，未遗留固定测试尺寸 |
| 实际平台网络下载 | 部分成功，详见下文；不将调度 PASS 当作每次平台请求均成功 |

在线测试沿用用户已授权的 Cookie 与 TikTok HTTP 代理。通过真实 Tauri 命令创建四个预览并提交四个下载，界面实时展示真实队列和状态，再通过设置控件改变并发数。未使用 mock 冒充平台结果。

四任务批次：TikTok 两项 completed，各 687985 字节；抖音一项 completed，466223 字节，另一项 failed/INVALID_INPUT。三个完成文件均经 FFmpeg 9.0.1 完整解码、零错误。失败任务释放名额，剩余队列正常完成。

对失败抖音作品单独新建任务复测一次，返回 NETWORK_ERROR，0 字节。两次失败的更细网络原因未确定；当前引擎也会将目标检查拒绝映射为 INVALID_INPUT，不能仅凭错误码归因于用户链接。未放宽网络边界、增加自动重试或扩大本卡修改范围。双平台均有本轮下载成功证据，但在线稳定性不记为全量 PASS。

本机脱敏证据：`.local/m2-queue-native-results.json`、`.local/m2-queue-native-recheck.json`、`.local/m2-queue-native-reduced.png`、`.local/m2-queue-settings.png`。降低上限后的截图显示“3 个下载中、1 个等待、并发上限 1”，这是保留已有下载的预期行为。

## 尚未验证

全新无 Python Windows、macOS Intel/Apple Silicon 与独立安装器仍为 NOT_RUN，留至 M3。Python 引擎生产代码和版本本轮未变，未重复执行与本卡无关的全量引擎测试。
