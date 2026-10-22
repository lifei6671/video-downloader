# YouTube、默认浏览器 Cookie 与本机工具

2026-10-14 用户授权新增。当前协议 v3、引擎 0.7.5，yt-dlp 2026.08.19、yt-dlp-ejs 0.8.0；历史验证记录按执行时版本保留。2026-10-21 的音轨、格式、进度、错误、分块与兼容规则见 [专项验证](ytdlp-processing-validation.md)。

## 行为

- 单个 YouTube watch、youtu.be、Shorts、embed 链接统一成 `watch?v=作品ID`；缓存保留 `v`，忽略追踪和播放列表参数。不支持播放列表、直播、任意站点或 DTK 的 YouTube 解析。
- 设置中新增 YouTube 独立 Cookie、代理、User-Agent，复用现有保存/显示/隐藏/清除流程。未修改时保存禁用，成功后用图标和 12px 主题色文本提示。
- “系统设置 → 平台账号 → YouTube → HTTP 分块下载”可选择继承上游默认、关闭分块或自定义。默认继承；自定义输入 1–1024 的整数 MiB，初始建议值 10 MiB。沿用“保存平台配置”，仅新任务使用新值，已经解析/排队/下载的任务保留配置快照。继承上游默认不等于关闭分块，速度需按实际网络和 CDN 测试，不承诺加速。
- 三平台手动 Cookie 优先；留空时，仅尝试操作开始时系统 HTTPS 默认浏览器。复用 yt-dlp 提取并按平台域名过滤，不写回 SQLite、不输出凭据、不启动浏览器、不尝试其他浏览器。不支持的浏览器、锁库、无法解密或没有此平台可用 Cookie 时提示限制；公开作品仍可匿名尝试，访问拒绝不会触发切换引擎。
- YouTube 规格优先最高分辨率，配对兼容的 MP4/M4A 或 WebM 音视频，通过 FFmpeg `-c copy` 合并。大小只在真实已知时相加；缺少所选规格明确失败。下载传输总字节与合并后文件大小分别校验，Rust 继续执行文件内容校验及不覆盖提交。
- 分离规格标记“需 FFmpeg”，合并期间显示状态并保留取消能力；合并单独限时 600 秒，不增加其他网络读取超时。取消/异常回收整个 Windows Job / Unix 进程组，包括 FFmpeg 与 JS 子进程。

## 工具检测

工具不随安装包内置；2026-10-18 起可在“系统设置 → 运行依赖”中检测并按需下载缺失工具，见 [运行依赖](runtime-dependencies.md)。YouTube 解析需要 Node.js 22+ 或 Deno 2.3+，分离音视频下载需要可执行的 FFmpeg。所有路径按独立 argv 传给子进程，支持空格和中文；不拼接 shell 命令。

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

## 浏览器 Cookie 状态与代理输入修复（2026-10-20）

- 默认浏览器复选框按已保存的手动 Cookie 是否为空恢复，保存成功后保持勾选；切换平台、离开设置或重新打开时沿用相同规则。取消勾选可编辑手动 Cookie，手动值留空仍表示使用默认浏览器，不增加另一份开关配置。读取失败时禁用 Cookie 编辑，保存其他设置仍传 `null` 以保留已有 Cookie。
- 抖音、TikTok、YouTube 保存手动 Cookie 时统一去除首尾空白及空行，成功后编辑框回显清理后的值；全空白按空值处理，恢复默认浏览器模式。中间内容保持原样，中间存在换行仍拒绝保存，不自动拼接；后端保存入口同样执行首尾清理，`null` 仍保留原值。
- 代理地址使用普通文本输入，提交前去除首尾空白；保留现有 URL 校验及系统凭据存储规则。系统 TUN 已接管网络时，应用的“直连”仍走系统路由。
- 用户反馈链接 `https://www.youtube.com/watch?v=a6cnYFcd7JQ` 的身份识别及本机 Deno 检测通过；首轮匿名直连解析得到 `RISK_CONTROL / network_target`，即 Google 验证重定向被既有边界拦截。后续源码与已安装的 0.7.2 引擎均匿名解析成功，返回 18 个规格；此结果不保证平台持续放行，也不代表实际媒体下载通过。默认 Edge Cookie 提取在复制占用中的数据库时得到 `PermissionError`，未成功取得 YouTube Cookie。没有修改 TUN 或代理配置；界面提示补充浏览器验证与切换 TUN/代理出口的处理建议，保留网络白名单与错误分类，不绕过验证。
- 回归入口：`tests/platform-settings.browser.js` + `tests/fixtures/platform-settings.html`，使用现有 Playwright CLI、Edge 和 `npm run dev`。仅使用虚构 Cookie 与隔离 IPC，覆盖三个设计视口、键盘切换、保存/刷新/平台切换、手动 Cookie、代理编辑及失败保留；不能代替原生 Tauri、真实浏览器解密或在线平台验收。

## YouTube 分块设置（2026-10-21）

`platform_settings.http_chunk_size` 使用可空 INTEGER：`NULL` 继承提取器默认、`0` 明确关闭、正数表示字节大小，必须是 1–1024 MiB 的整数倍。旧表按原方式添加一列，旧记录默认继承，Cookie/代理/UA 保留。保存接口中省略字段保留原值，显式 `null` 恢复继承；不允许其他平台设置此字段。

Rust 把 YouTube 值纳入既有任务配置快照，Python 验证后交给固定版本 yt-dlp 的原生 HTTP 下载器；显式关闭/自定义同时覆盖已选音视频流的 `downloader_options`，避免提取器的默认值覆盖用户选择。不修改规格 ID、音轨、续传、网络安全边界或文件提交逻辑。显式设置优先于旧探针环境变量，继承仍保留提取器默认，不受该变量影响；未携带新字段的旧探针仍兼容环境变量。引擎/bridge/握手更新到 0.7.5，协议仍 v3；需重建 sidecar，解析缓存按现有版本隔离，历史任务和成功文件保留。

回归入口增加 `tests/youtube-chunk-settings.browser.js`：三视口、键盘操作、三模式保存/恢复、MiB 转字节、非法输入、切换平台保留草稿、保存中禁用、保存失败重试及 Cookie 读取失败保留。`tests/test_http_streaming.py` 经 GuardedRH 和真实 HttpFD 验证继承、关闭及 1/3 MiB 自定义均下载同一规格且字节一致；均为本地受控 HTTP，不是在线 CDN 性能数据。

## 代理回显与设置顶部固定（2026-10-21）

三个平台的代理输入框保存后保留真实地址，重新打开设置时通过 `get_profile_proxy` 专用命令读取；含认证信息的完整地址从现有系统凭据存储读取，只返回设置编辑器，不加入普通 settings 摘要、任务事件或数据库。没有修改代理时提交 `proxy=null` 保留凭据，选择直连并保存才清除；切换标签或未保存的直连/代理模式不丢失草稿。读取中或失败时禁用代理编辑和模式切换，仍可保存其他字段且保留原代理；失败原因持续显示。

系统设置的分类标题与平台 Tabs 位于滚动区外，只有当前平台的配置面板滚动；左侧分类导航固定可见，保留方向键切换标签和表单键盘操作。下载设置与运行依赖同样只滚动分类内容，避免内外两层滚动。

回归入口 `tests/platform-settings-layout.browser.js`，覆盖 1440×900、1280×800、1024×680 的完整认证代理回显、保存/重新打开、只改 UA 保留凭据、直连清除、固定标题/Tab/导航、滚动与键盘切换，以及读取失败/延迟读取/保存失败。所有凭据均为隔离 fixture；不代表真实系统凭据可用性或原生 GUI 人工验收。既有平台设置、YouTube 分块、tdl 路径选择和平台任务回归同步执行。Python 引擎保持 0.7.5 / JSONL v3，本次不修改下载参数、网络边界或任务提交。

本次实际验证：`npm run build` PASS；`cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1`（`CI=true`）104 passed / 2 ignored；`cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` PASS。Playwright CLI + Edge：新增代理/布局 6 组、既有平台设置 10 组、分块 6 组、tdl 路径 11 组、任务平台 8 组全部 PASS。任务平台首轮被文档修改触发的 Vite 页面重载中断，冻结文件后重跑 8 组通过。

Windows 预览 `output/Yingqu-1.7.1-preview.20261021-settings-win64/` 已通过 `npm run tauri -- build --features desktop --config D:/wx_lifeilin/github.com/lifei6671/video-downloader/.local/proxy-settings-header-20261021/tauri.preview.conf.json --no-bundle` 编译；完整资源与上一包逐文件一致的 Python sidecar 直接复用，未重新执行 PyInstaller。最终预览的 `scripts/check_packaged_engine.py` 隔离握手 PASS，仍为 0.7.5 / yt-dlp 2026.08.19 / v3。GUI SHA-256 `0ede1264db48f07a79c279d011eced1088d7b42dd35723673b1c0e77bc5f476c`，包内附来源和全文件校验清单。日志与三视口截图在 `.local/proxy-settings-header-20261021/`。Python 源码未变，本次未重跑 Python 测试或平台下载；真实系统认证代理读取、原生 GUI 人工验收与 macOS 验证仍 NOT_RUN。没有提交、推送、发布或退出正在运行的旧预览。
