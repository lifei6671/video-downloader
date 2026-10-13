# M2 第一张任务卡验收

日期：2026-10-13。范围：桌面单任务真实闭环；沿用 yt-dlp 2026.08.19 与引擎 0.2.1 抖音详情补丁。用户追加要求为亮色主题、所有 UI 控件采用 shadcn/ui，以及 Cookie 直接保存 SQLite。

## 已实现

- Tauri 2 原生窗口 + React/TypeScript，shadcn/ui 官方 radix-nova 组件；按钮、输入、侧栏、卡片、进度、提示、空状态、确认弹窗和设置控件均来自官方组件，页面仅组合业务与布局。亮色 token 集中映射 DESIGN.md，不依赖 CDN。
- 单条分享链接解析、作品预览、开始下载、真实进度、取消及完成记录。Rust 是任务状态唯一来源；下载独占进程，验证引擎退出及文件后提交，完成记录成功落库才发布完成。
- 分平台 Cookie/UA/代理与下载目录；Cookie 在本机 SQLite 中直接保存，保存后不回填。代理密码仍用系统凭据，未带密码代理不依赖系统凭据。
- SQLite 恢复、同一数据目录单实例锁、未完成任务中断标记、提交意图恢复、暂存清理、文件缺失显示。重启不会自动下载，不提供断点续传。

## 验证结果

| 项目 | 结果与证据 |
| --- | --- |
| 前端类型与生产资源构建 | PASS — `npm run build`；官方组件的 use client 指令在 Vite 构建中忽略，不影响产物 |
| Rust 核心 | PASS — `cargo test --manifest-path src-tauri/Cargo.toml --lib`，10 项；真实子进程 fixture 覆盖取消/停止写入/清理、文件提交、恢复、SQLite Cookie 与配置隔离 |
| Rust 静态检查 | PASS — `cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` |
| 桌面编译 | PASS — `cargo build --manifest-path src-tauri/Cargo.toml --features desktop --bin yingqu`，Windows debug 原生应用 |
| 引擎回归 | PASS — 构建 m1-probe 后 `.venv/Scripts/python.exe -m pytest -q`，74 项 |
| 原生窗口双平台在线 | PASS — 已授权测试凭据，UI 保存设置、输入分享链接、解析预览、点击下载；下面两项均 completed |
| 文件完整性 | PASS — FFmpeg 9.0.1 对两个新文件执行 `-v error -i <file> -f null -`，零错误 |
| 原生 UI 视口 | PASS — 1440×900、1280×800、1024×680 的首页/任务/已完成/设置，共 12 个页面状态无横向溢出；小窗口侧栏折叠、设置改单列 |
| 键盘和失败状态 | PASS — Enter 解析、非法输入持续显示错误、Enter 打开取消弹窗、Escape 关闭并恢复焦点、确认取消；发现的焦点丢失已修复 |
| 设置公开契约 | PASS — Cookie 输入保存后清空，get_settings 仅返回配置状态，无 Cookie/凭据引用；抖音直连与 TikTok 代理隔离 |
| 原生重启 | PASS — 正常关闭退出后重新启动，2 条 completed 记录与两平台配置状态保留 |
| 浏览器独立预览 | 不作为原生下载证据；本次 UI 自动化通过 WebView2 CDP 操作真实 Tauri 窗口，未用 mock 冒充在线结果 |

下载结果（应用的系统 Downloads/Yingqu 目录）：

| 平台/作品 | 字节 | SHA-256 |
| --- | ---: | --- |
| 抖音 7682077475388978298，直连 | 466223 | `29c08afc224492d400a3dac16039cad34e1cb11632da19d4b4e7042ce208086a` |
| TikTok 7676209499589446945，HTTP 代理 | 687985 | `27a37a403642d5ceae29a38576d776e7e2cb67840df794017cccd0c144be4c30` |

本机原始证据在忽略目录 `.local/m2-native-results.json`、`.local/m2-layout-results.json`、`.local/m2-light-*.png`、`.local/m2-1440-*.png`、`.local/m2-1280-*.png`、`.local/m2-1024-*.png`。截图及结果不包含 Cookie 或代理密码。测试用 Cookie 文件不进入仓库。

## 明确边界

- 本卡只实现单任务。并发默认 2/上限 3、等待队列、手动重试、移除记录和二次确认删除视频留到后续卡，不改变首版契约。
- 当前是开发桌面程序，引擎使用仓库中已有 PyInstaller onedir；不是独立安装器。全新无 Python Windows：NOT_RUN，按用户要求移到 M3 待验证；macOS Intel/Apple Silicon：NOT_RUN。
- 含密码代理的系统凭据写入、系统目录选择器的人工交互未在本次自动化覆盖；普通无密码 HTTP 代理已真实验证。
- 公开作品解析成功不证明 Cookie 登录身份有效；本卡没有“测试 Cookie”按钮。
