# YouTube、默认浏览器 Cookie 与本机工具

2026-10-14 用户授权新增。协议 v3，引擎 0.6.0，yt-dlp 2026.08.19，yt-dlp-ejs 0.8.0。

## 行为

- 单个 YouTube watch、youtu.be、Shorts、embed 链接统一成 `watch?v=作品ID`；缓存保留 `v`，忽略追踪和播放列表参数。不支持播放列表、直播、任意站点或 DTK 的 YouTube 解析。
- 设置中新增 YouTube 独立 Cookie、代理、User-Agent，复用现有保存/显示/隐藏/清除流程。未修改时保存禁用，成功后用图标和 12px 主题色文本提示。
- 三平台手动 Cookie 优先；留空时，仅尝试操作开始时系统 HTTPS 默认浏览器。复用 yt-dlp 提取并按平台域名过滤，不写回 SQLite、不输出凭据、不启动浏览器、不尝试其他浏览器。不支持的浏览器、锁库、无法解密或没有此平台可用 Cookie 时提示限制；公开作品仍可匿名尝试，访问拒绝不会触发切换引擎。
- YouTube 规格优先最高分辨率，配对兼容的 MP4/M4A 或 WebM 音视频，通过 FFmpeg `-c copy` 合并。大小只在真实已知时相加；缺少所选规格明确失败。下载传输总字节与合并后文件大小分别校验，Rust 继续执行文件内容校验及不覆盖提交。
- 分离规格标记“需 FFmpeg”，合并期间显示状态并保留取消能力；合并单独限时 600 秒，不增加其他网络读取超时。取消/异常回收整个 Windows Job / Unix 进程组，包括 FFmpeg 与 JS 子进程。

## 工具检测

不内置、不下载工具。YouTube 解析需要 Node.js 22+ 或 Deno 2.3+，分离音视频下载需要可执行的 FFmpeg。所有路径按独立 argv 传给子进程，支持空格和中文；不拼接 shell 命令。

| 系统 | 当前检测来源 |
| --- | --- |
| 通用 | 当前进程 PATH、`~/.local/bin`、`~/bin`、激活的 Conda 环境 |
| Windows | 注册表中最新用户/系统 PATH、WinGet Links 与 FFmpeg portable 包目录、Scoop shims/FFmpeg current、Chocolatey、Program Files 常见目录；Node 另支持 NVM_SYMLINK，Deno 支持 `~/.deno/bin` |
| macOS | Apple Silicon `/opt/homebrew` 与 Intel `/usr/local` 的 bin/opt/Cellar（含版本化、keg-only 与 ffmpeg-full）、`HOMEBREW_PREFIX`、MacPorts `/opt/local/bin`、`/etc/paths` 和 `/etc/paths.d`、常见 Nix/Conda 用户目录；Node 另支持 nvm 安装目录 |

FFmpeg 实际运行 `-version` 后才被采用；Node/Deno 同样逐候选验证最低版本，旧 PATH 中的低版本不会遮挡其他安装目录中的新版。损坏链接、不可执行、架构不兼容、超时或非 FFmpeg 同名文件会跳过。未找到可用工具才提示安装。不会递归扫描整个磁盘，也不会运行 shell 启动脚本；任意自定义目录需加入系统 PATH。工具权限/架构问题与未安装一样，都可能导致“未找到可用工具”。

## 验证边界

- Windows：已确认旧进程 PATH 漏掉了注册表中的 WinGet FFmpeg 安装目录，修复后实际找到并执行本机 `N-126374-g089a48eb36-20260831`。真实 FFmpeg 生成本地音视频 fixture，经真实 yt-dlp HTTP 分流下载、合并及 ffprobe 验证，输出同时保留 `mpeg4` 视频和 `aac` 音轨，无转码。
- 公开平台：源码与 Windows onedir 经本机已有代理解析 `https://www.youtube.com/watch?v=YE7VzlLtp-4`，均成功返回 17 个规格，最高 1080p，打包引擎正常退出。在线探针使用隔离的无凭据设置/虚构 Cookie，没有使用真实 Cookie；公开解析不证明登录身份有效。旧 yt-dlp 测试视频 `BaW_jenozKc` 在线返回上游错误，未将其失败当作适配通过。
- 离线：覆盖链接/身份、格式与凭据隔离、默认浏览器异常、工具检测、合并进度/终态、进程后代回收（含 Windows 后代持有禁止删除共享的媒体文件句柄时，取消后立即清理 staging）、设置持久化与缓存别名；具体命令沿用开发文档与 CI。
- UI：Edge + 隔离 Tauri IPC fixture，检查三个平台设置、保存状态、提示、规格需求、合并状态与 1440×900 / 1280×800 / 1024×680 布局。此项不等于 Tauri 原生运行。
- Windows onedir 引擎启动可离线验证；不代表全新 Windows 安装包验收。YouTube 平台真实媒体下载、真实浏览器解密与登录身份、macOS Intel/Apple Silicon 的本机工具/打包运行仍需分别验收，不能用 Windows 或 mock 结果代替。

本轮验证：`python -m pytest -q` 165 项通过；Rust 完整运行 34 项通过；`cargo clippy --locked --features desktop --all-targets -- -D warnings`、`npm run build`、DTK vendor check、PyInstaller 打包和无 Python PATH 启动检查通过。前端保留已有 use-client/sourcemap 构建告警。

参考：[yt-dlp EJS](https://github.com/yt-dlp/yt-dlp/wiki/EJS)、[downvid 固定源码](https://github.com/yxxbc/downvid/tree/b1ef96800f1bb999bf63bee6b9a0ad98996494ea/electron/utils)。downvid 按安装顺序选浏览器，本项目按用户要求识别系统默认浏览器。

## TUN 环境合法链接被误报的修复（2026-10-14）

用户报告 `https://www.youtube.com/watch?v=Ql7DR8YKwSg` 显示链接不支持。源码及正在使用的打包引擎均能识别此链接；本机 TUN DNS 同时返回 IPv4 和私有 IPv6 `fc00::63`，被旧版公网 IP 检查拦截并误报 `INVALID_INPUT`。用户明确要求不判断 DNS 返回 IP，TUN 私有地址应交给系统路由。

现移除 IP 公网属性检查与对应无效测试，不新增 TUN 开关或检测。平台域名/协议/端口及逐跳重定向检查继续保留；直连仍只解析一次，按系统返回顺序连接，失败 socket 关闭后继续下一地址。显式代理配置与平台隔离保持不变，不修改用户设置。

实际验证：修改后源码及重建后的 Windows onedir 均在当前 TUN 环境、不设置应用代理即可解析此链接，返回 18 个规格，打包引擎正常退出。探针使用虚构 Cookie，不读取真实浏览器凭据，未执行平台媒体下载。

解析任务错误、全局操作错误及 Cookie 警告统一为无边框、透明背景、icon + 12px 主题色文本，保留可访问的 alert 语义和持续可见的失败原因。Edge 隔离 IPC fixture 验证三个设计视口、键盘提交、每条提示一个图标、0px 边框及实际字体；不等于 Tauri 原生运行。

已启动的 `yingqu.exe` 与其资源不会随源码更新；退出应用后，重新构建桌面发行产物才能应用本次 UI 和 TUN 修复。Python 引擎已在 `python-engine/dist/video-engine/` 从最新源码重建并通过无 Python PATH 启动检查。最终完整 Python 166 项通过，前端类型检查与构建通过；原生 Tauri GUI 更新仍需退出后重建，macOS 实机未运行。
