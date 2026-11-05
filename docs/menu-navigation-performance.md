# 菜单切换性能验证

2026-11-05，Windows 本机。范围为菜单切换卡顿的测量、优化、编译和启动；保留前轮五项缺陷修复，未提交或推送。

## 测量与定位

在实际 Tauri WebView2 窗口中经本机 CDP 采样，优化前后均为 Rust Debug + Vite 生产前端。使用同一套本机数据，251 条下载记录、397 条备份操作；不输出记录正文、凭据或 IPC 参数。

`tests/menu-navigation-performance.mjs` 按固定顺序循环四个主菜单 8 轮，测量程序触发点击到两次 `requestAnimationFrame` 回调的间隔。热切换统计排除第 1 轮，剩余每菜单 7 个样本；P95 在该样本量下就是最大值。这是 WebView 渲染响应指标，不等同于操作系统输入延迟或屏幕实际呈现时间。未测试 Release，也未做跨硬件推断。

| 原生指标 | 优化前 | 最终版本 |
| --- | ---: | ---: |
| 返回下载任务页，中位数 | 62.8 ms | 37.9 ms |
| 返回下载任务页，P95 | 72.5 ms | 46.6 ms |
| 进入系统设置，中位数 | 25.1 ms | 24.2 ms |
| 进入关于软件，中位数 | 8.3 ms | 13.7 ms |
| 进入下载作品，中位数 | 10.3 ms | 14.6 ms |
| 读取备份操作列表，3 次中位数 | 2480.6 ms | 111.2 ms |
| 操作列表序列化长度 | 20,972,856 JSON 字符 | 111,894 JSON 字符 |

任务页热切换中位数降低约 40%，操作列表读取中位数降低约 95.5%，序列化长度降低约 99.5%。其他轻量页面处于一至两帧量级波动，没有证明它们都变快。首次进入任务页本次为 73.3 → 109.5 ms，**没有证明首访提速**。序列化长度使用 `JSON.stringify(...).length`，不是精确 UTF-8 字节数；早期原始日志该字段名为 `bytes`，最终脚本已更正为 `jsonChars`。

CPU 采样显示任务页反复挂载时，DOM 插入及 Radix Presence 的样式/动画名读取是主要热点之一。原同步 `list_backup_operations` 同时读取、反序列化并传回全部内部 payload，包含封面等大对象；该命令在原生 UI 线程执行，实际单次耗时为 2.4–2.5 秒。前端每秒轮询这一接口，会持续放大卡顿。

## 实现

- `scheduler::operations_for_display` 在 SQLite 内只提取前端使用的七个字段：`completed_bytes`、`transfer_started`、`local_cleanup_done`、`archive_next`、`cleanup_dependency`、`cleanup_error`、`cleanup_blocked`。保持操作顺序、身份、状态、阶段、错误、尝试次数以及布尔值/零值；worker 继续读取完整持久化 payload。
- `list_backup_operations` 通过 `spawn_blocking` 执行查询，避免查询及投影工作占用原生 UI 线程。没有新增缓存、依赖或数据库迁移。
- 任务页首次进入时仍按原规则加载 30 条，随后保留已经访问的分页内容与 DOM 身份。隐藏时跳过父级更新带来的页面协调，返回时使用最新状态和回调。任务列表自身滚动保留，普通页面换页时外层滚动归零。未改变后台同步与下载生命周期。

## 验证

| 检查 | 结果 |
| --- | --- |
| Rust `display_projection_keeps_ui_state_and_leaves_recovery_payload_intact` | PASS，覆盖大 payload、空投影、bool/0、错误、顺序及原数据不变 |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASS |
| `npm run build` | PASS |
| `npm run desktop:build -- --debug -- --locked` | PASS，含打包引擎检查 |
| `tests/menu-navigation-browser.mjs` | PASS，3 视口 × 2 布局，共 6 场景 |
| `tests/backup-space-visibility-browser.mjs` | PASS，12 场景 |
| `tests/backup-delete-flow-browser.mjs` | PASS，12 场景 |
| `git diff --check` | PASS |

菜单回归覆盖首次延迟挂载、DOM 身份保留、隐藏期间任务更新/删除、返回后的最新标题、任务列表滚动、普通页面滚动复位、键盘切换及右键菜单。浏览器使用 synthetic IPC；原生采样使用真实本机状态，未主动执行在线下载、删除、恢复或账号变更。macOS、Release 和安装包验收 `NOT_RUN`。

本地证据：`.local/menu-performance/native-before.json`、`native-final.json`、各测试日志与 `launched.json`。采样入口关闭后已正常重新启动 Debug 应用；前端仍为生产构建。

## 交付复核

Review mode：`CHILD_AGENT`；Strategy：`FULL_SCOPE`；Verdict：`APPROVED`；Gate：`PASSED`；Coverage：`COMPLETE`；Freshness：`FRESH`；Repair rounds：1。无剩余 P0/P1/P2。

审查仅覆盖本轮 4 个实现文件、2 个测试文件和 `DESIGN.md`，以前轮交付后的工作区快照作为基线。审查中发现并修复了跨普通页面继承滚动位置的问题；测试曾复现 192px 泄漏。

目标清单 `.local/menu-performance/target.json` 的 SHA-256：`A571748FD3172F69C7C0A466BFFD5A44878D4CBBF08E638902C93894D0505633`；独立重算 7 文件，0 处不匹配。本记录为纯文字证据，不属于可执行目标。
