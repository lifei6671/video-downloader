# Telegram 原始视频 HTTP 实验模式

映取调用用户自行安装的 tdl，**不绑定版本号**，v0.20.4 是已进行真实验证的基线，不分发或自动安装 tdl，不使用 Telethon，不解析终端进度条。用户点击后可在界面内扫码、输入 2FA，由隐藏伪终端中的官方 tdl 创建独立会话；不导入、迁移或覆盖既有登录。抖音、TikTok、YouTube 继续使用原有 Python 引擎。

## 启用与使用

1. 用户在映取外自行安装 [tdl](https://github.com/iyear/tdl/releases)。尚未配置独立会话时沿用 tdl 默认会话；停止其他使用同一会话的 tdl 命令，避免上游存储锁冲突。
2. 打开「系统设置 → 运行依赖」中位于 JavaScript 运行时下方的 tdl 卡片，留空自动检测；点击路径输入框右侧的文件夹图标，使用系统文件选择器选择 tdl 可执行文件，选中后自动保存并立即执行 `tdl version`，同一路径也会重新检测。取消选择不更改配置；也可手动填写完整绝对路径或清空后按 Enter 保存并检测。不再提供底部「检测 tdl」和「保存 tdl 路径」按钮。Windows 选择器筛选 `.exe`，macOS 允许选择无扩展名可执行文件；检测失败如实显示不可用，路径仍保留，保存失败则保留原配置。查找覆盖进程 PATH、GOBIN、各 GOPATH/bin、`go env -json GOBIN GOPATH` 的配置、默认 ~/go/bin、Windows 注册表环境及 Go/WinGet/Scoop/Chocolatey 常见位置，以及 macOS Homebrew/MacPorts、paths 文件和常见用户 bin 目录；不改变 PATH 或 Go 配置，不下载工具链。
3. 切换至「系统设置 → 平台账号 → Telegram」，阅读并确认风险提示，勾选 HTTP 实验模式并保存。默认关闭；检测成功仅证明可执行程序能正常返回版本信息，不证明 serve/login 能力、已经登录或服务安全。
   默认会话不可用时，点击同页“登录 tdl”，使用 Telegram 扫描弹窗内二维码；官方提示 2FA 后在密码框输入两步验证密码，提交即清空。登录成功后解析和下载改用映取独立会话。关闭弹窗、取消登录或等待超过 5 分钟均视为失败，不切换原配置；密码错误可重新扫码。
4. 粘贴 `https://t.me/<username>/<message>` 或 `https://t.me/c/<channel>/<message>`，引擎选自动或 tdl。只接受单条消息，不支持相册、讨论区、带 query/fragment 的链接、话题及批量消息。
5. 下载中可继续解析其他消息：当前服务已确认的用户名、已有解析缓存确认的频道用户名及数字频道链接复用同一个服务。用户名映射从当前修订的已有消息别名和数字身份提取，支持 t.me / telegram.me、用户名大小写及同频道不同消息，不新增映射表或认证会话；新消息仍执行 HEAD 取得自己的真实元数据。首次遇到未确认的新用户名频道时提示当前 Telegram 服务被占用，下方提供“加入待下载队列”，用户点击后只保存规范链接并进入现有 FIFO 队列，不立即解析，也不占等待期间的下载名额。前面的 Telegram 操作结束且有下载名额后自动解析并下载；可取消，失败保留原因和链接，重试沿用任务 ID 与创建时间。
6. 预览只列一个 `original` 规格。文件大小、类型和清理后的文件名来自 HEAD；时长、尺寸、编码未知则为空。支持上游明确返回的 MP4、WebM、QuickTime MOV、Matroska MKV，不转码或改写字节；其他类型明确拒绝。
7. 视频完整下载后，使用已有本机或应用 runtime 中的 FFmpeg 在前 30 秒内从第一帧开始每约 0.5 秒采样，跳过近乎全黑的画面，取第一张可用帧生成最长边不超过 1280 像素的 JPEG 封面，不改写原视频。验证封面类型、大小和固定暂存文件名后，只写入同一作品的现有 SQLite 封面缓存，不在下载目录生成封面图片；抽帧暂存随任务清理。任务、完成记录和后续缓存解析复用原封面展示，正常重启以及已有提交证据的中断恢复均可读取缓存。tdl HTTP 接口本身不提供封面，首次解析仍可能没有封面。黑帧筛选使用 FFmpeg `blackframe` 的像素比例：8 位亮度低于 24 的像素达到 98% 时跳过；不对原视频调亮或转码。前 30 秒内没有可用画面或达到现有超时时，保留默认占位图，视频仍正常完成。已缓存的历史封面不自动重抽。

移除 Telegram 任务记录时，如果没有其他任务或预览仍引用同身份的缓存封面，清空对应封面 BLOB；只清理封面，不删除可复用元数据和链接别名，也不删除本地视频。清理和记录删除使用同一 SQLite 事务，失败则整体回滚。多条记录共用的封面保留至最后一个使用者移除，频道之间独立。单独删除本地视频仍保留记录和封面；旧版已经保存到视频旁的封面文件保持原样，不扫描下载目录做历史迁移或删除。

抽帧由内置 Python sidecar 的本地 `extract_cover` 操作执行，仅接受任务暂存中的固定媒体文件名，只开放本地文件协议和已支持的容器，不访问 Telegram 或登录存储。FFmpeg 执行上限 10 秒，整个 sidecar 操作上限 20 秒（包括启动及工具检测）；抽帧期间仍占原下载槽位，取消和应用退出沿用原进程组 / Windows Job 回收与暂存清理。缺少 FFmpeg、解码失败、超时、无效封面或封面缓存写入失败均不改变已成功下载的视频结果；不自动安装 FFmpeg，不对旧完成记录补抽帧。封面操作需要同步重建内置引擎 0.7.1，JSONL 协议仍为 v3。

设置保存在现有 SQLite 中。tdl 路径及启用状态在每个任务开始时取快照；已进入操作的任务沿用快照，关闭实验模式阻止新的解析/下载提交，不自动中止已提交任务。可在任务内取消。依赖版本检测使用临时 `--storage type=bolt,path=...`，避免 tdl 根命令钩子触及用户的存储迁移；尚未配置独立会话时，实际服务使用用户已有默认配置/会话；平台账号登录成功后改用映取独立存储及 `yingqu` 命名空间，不复制或重写旧登录数据。tdl 自身仍可能维护其正常运行所需的会话数据与日志。

## 已知服务暴露风险

核对基线：tdl v0.20.4，commit `9d7d49eef2bcb04da720c26e33598c49c68b9ddd`。

- [serve.go](https://github.com/iyear/tdl/blob/9d7d49eef2bcb04da720c26e33598c49c68b9ddd/app/dl/serve.go) 使用 `Addr: fmt.Sprintf(":%d", port)`，监听所有接口。
- `/{peer}/{message:[0-9]+}` 路由可重新解析任意请求的 peer/message，没有访问令牌或输入消息白名单；首页只有一条链接，不代表其他消息不可访问。可连接端口的其他设备可能读取当前账号有权访问的媒体。
- **只访问 127.0.0.1、使用临时端口或隐藏链接均不构成网络隔离。** 仅在可信网络中使用，并自行通过系统防火墙限制外部访问。映取不修改系统防火墙，也不静默添加代理。
- 映取的客户端边界只保护自身请求：无代理、无自动重定向，只请求当前服务的 `127.0.0.1:<port>` 首页及每个请求独立绑定、经过数字身份校验的媒体路径。它不能修补上游对其他网络连接的开放。

`TdlStatus` 只返回实际可执行路径及版本，不宣称验证服务或认证能力。依赖检测不检查 help，不按版本号拦截；开始媒体操作时验证真实 serve 命令参数，登录时验证 QR 命令能力，HTTP 和认证交互仍按实际响应校验。若上游新增安全能力，须先核对真实 flag/HTTP 契约，再更新启动参数和请求校验；当前不拼接任何猜测的安全参数，不因版本更新而宣称安全。若上游仍未修补，继续保留明确确认的实验模式。

## 生命周期与数据契约

- 同一会话的重叠操作复用一个 `tdl dl -u <url> --serve --port <port>` 服务和 HTTP 客户端；只在服务启动/借用与最后回收时持锁，媒体流期间不持会话调度锁。最后一个操作结束即停止服务，不保留闲置监听。先以临时监听器选择可用端口，然后启动并轮询首页；服务提前退出或端口竞争明确失败，不改用其他站点或代理。服务锁等待、启动（含首页与 HEAD）及复用服务的 HEAD 各限时 30 秒，读取闲置超时 60 秒，媒体操作最长 6 小时。
- 启动时首页必须恰有一个符合 v0.20.4 模板的媒体链接。数字频道 ID 以 tdl 解析结果为权威；消息 ID 必须与输入一致，私有频道链接还核对频道 ID。得到规范身份 `<channel>_<message>` 和 `https://t.me/c/<channel>/<message>`，禁止任意 HTTP 地址/路径、重定向、多条链接及跨消息结果。
- 上游首页静态列出启动消息，后续媒体响应不提供用户名对应的数字频道 ID。当前服务只绑定由启动首页确认的用户名；后续同名频道请求转换为该数字频道路径，新数字链接直接按频道/消息身份生成路径。已有解析缓存中的用户名映射可用于其他频道的新消息；缓存映射仅在服务已运行时辅助选择数字路径，服务空闲时仍以原公开链接启动并重新确认身份。没有有效映射时返回 `TDL_CHANNEL_ID_REQUIRED`，允许用户将该链接加入延后解析队列。下载可用已验证缓存别名启动 tdl，但新首页必须仍匹配任务原有数字身份；服务已运行时，下载使用任务已有的规范数字身份。若启动时证明公开别名已指向其他数字频道，本次下载明确失败；数字身份与缓存保留，清除该用户名仍指向旧频道的消息别名；之后可用数字链接重试，不自动切换或重试。新解析确认用户名已改属时替换旧频道别名。缓存修订失效、身份不符或映射冲突时不使用该映射。每个 HTTP 请求独占不可变路径，后续解析不会改写正在下载的目标。
- 上游媒体缓存键直接拼接 peer 与 message，不带分隔符。映取记录当前服务请求过的数字路径，拒绝 `100/12` 与 `1001/2` 这样的缓存键碰撞，避免返回另一条消息。客户端边界不能约束其他设备对上游开放服务发起的请求。
- 未解析的 Telegram 排队项使用现有 `queued` 状态，`video=null`，任务快照的 `telegram_link` 只保存校验后的规范单消息链接，不保存分享文案。开始处理时预留一个下载名额并进入 `parsing`；成功后保留原队列顺序再下载。等待项让此前的 Telegram 解析/下载先结束，阻止后入队的 Telegram 下载反复占用服务；其他平台可继续使用空闲名额。重复作品解析后停在 ready，须用户确认才再次下载。取消和退出沿用原有回收；重启将未完成项标为 INTERRUPTED，不自动续传，可凭保存的链接手动重试。
- Telegram 与现有平台共用全局 1–3 下载槽位，可同时下载多个 Telegram 视频，超出容量仍按现有 FIFO 等待；解析不占下载槽位。服务持有会话互斥保护以排除登录及其他 Core 的服务，同一会话仍最多一个服务进程。此锁不控制用户在映取外启动的 tdl。
- HEAD 要求真实正 Content-Length、支持的 Content-Type 和 Content-Disposition。文件名仅作清理后的元数据；实际暂存名固定为 `video.<container>.part`，目标文件名由 Rust 的频道/消息身份与任务 UUID 生成，原始文件名永不参与路径。
- 下载借用当前服务（没有时才启动），核对身份和 HEAD，GET 再核对长度、类型、文件名，禁止 Content-Encoding。上游同一服务会缓存媒体信息；映取仍重新执行 HEAD/GET 校验，无法使上游重新解析已缓存消息。HEAD 元数据变化时明确失败并失效对应缓存，后续重新解析/重试获取新值。HTTP 200 或覆盖完整文件的 `206 bytes 0-(N-1)/N` 可接受，局部 Range 拒绝；不提供断点续传。
- 每块字节写入成功后才计入进度，约 250ms 发布 bytes/total/speed/ETA；速度取近期采样窗口。UI 在提交前最多显示 99%。HTTP、实际字节数、文件头、本地文件与现有不覆盖提交、SQLite 完成记录全部成功后才完成，错误网页、截断和提前断开均失败。
- tdl 是长驻服务；单个操作成功/取消/错误/超时只结束自己的 HTTP 请求，其他借用操作继续。最后一个操作退出时主动停止并等待进程回收，再发布终态或提交文件；**不以被停止的 tdl 退出码判定下载成功**。应用退出取消全部请求并等待最后一次回收，再清理暂存；Windows 复用现有 kill-on-close Job，Unix 使用独立进程组。上游使用进程级 context，关闭客户端请求不保证立即取消其内部 Telegram RPC；只在最后一个操作结束时终止整个服务。
- SQLite 缓存白名单原始文件元数据、规范身份/别名和抽帧封面，不缓存临时 HTTP 端口或 URL。缓存按独立 `tdl-0.20.4-http-v1` 修订隔离，该字符串是应用缓存协议修订，不用于比对本机 tdl 版本；可复用 ready 预览和记录按现有机制恢复，未完成操作中断恢复，不自动续传。下载每次重新检查权限和媒体；历史移除保留解析元数据，仅在无其他引用时释放对应封面。

## 验证方法与证据边界

### 登录契约

- 上游 `tdl login` 默认使用 desktop 导入且明确警告覆盖命名空间。映取只调用 `login --type qr --storage <新建 Bolt 目录> --ns yingqu`；每次尝试目录唯一，不读取现有数据库。版本检测仍使用临时存储；登录前检查 `login --help` 的 QR 能力。
- 通过 `portable-pty` 直接启动官方可执行文件，不经过 PowerShell/Terminal/shell。Windows 使用 ConPTY 和现有 kill-on-close Job，Unix 使用 PTY 及独立进程组。`vt100` 仅在 Rust 内处理终端输出、游标查询和重绘；只向前端返回当前二维码布尔像素矩阵与登录阶段，不转发原文、账号身份或二维码 URL。官方交互解析基于已验证的 v0.20.4 格式，不按版本号阻止其他版本；不符合实际二维码或密码提示格式时无法继续，明确失败或超时，不作为通用终端或命令执行入口。
- 只有官方显示 `Enter 2FA Password:` 后才能提交密码，需匹配当前随机登录 ID；重复、迟到、非提示阶段、空值或控制字符输入拒绝。密码仅通过专用 IPC 和短时内存传给该进程，Rust 使用 `Zeroizing` 容器，前端在提交/关闭/终态清空输入，不进入配置、SQLite、普通事件或日志。原来的外部终端、PowerShell/AppleScript 启动路径已删除。
- 登录、解析、下载共用会话互斥锁。已有活动 Telegram 任务（含 ready 预览）须先取消，登录期间拒绝新 Telegram 解析/下载；其他平台保持原行为。
- 仅官方登录命令退出成功后保存独立会话路径至 SQLite。登录失败、取消、超时、应用退出确认回收后删除本次目录，原配置及以前创建的会话目录保留。停止时回收进程、关闭 PTY 输入/主端并等待读取线程结束，再释放会话锁；若不能确认回收则返回 `TDL_STOP_FAILED` 并保留该次目录。成功切换清理 Telegram 缓存。保存普通设置不能改变该路径。后续 `dl` 始终传同一 `--storage` 和 `--ns yingqu`。配置已保存不意味着账号持续有效，仍以实际请求为准。
- 独立认证数据属于本机 tdl 存储，不进入 SQLite、事件、普通日志或安装包；SQLite 仅存路径。此功能不下载或内置 tdl，也不解除 HTTP 实验模式的服务暴露风险。

本地独立模拟服务：`tests/fixtures/tdl_mock.py`（Python 标准库，仅监听 loopback，无 Telegram 库或凭据），由 Rust 测试启动和回收。测试构建仅对这份专用 fixture 使用 Python 启动前缀；生产构建直接执行本机 tdl。

```powershell
cargo test --locked --manifest-path src-tauri/Cargo.toml tdl_bridge -- --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings
npm run build
```

模拟覆盖 HEAD 元数据与 Unicode 文件名、多个文件大小、分块写入与真实字节进度、GET/HTTP 错误、截断、错误文件头与类型、Range、重定向/非法路径、取消与端口关闭、启动超时/冲突、共享服务并行解析/下载及请求独立取消、任务队列、去重、SQLite 缓存及重启恢复、原始文件安全提交和退出清理。

真实已登录 tdl 在线解析/下载需要用户另外提供并授权测试环境。模拟测试不证明真实 Telegram 访问、tdl 实际二进制或登录状态可用；Windows 本地编译/测试、真实 GUI、安装包、macOS Intel/Apple Silicon 分别记录，不互相替代。当前执行结果见本次交付说明；未执行的平台/在线项不得记为通过。

### 2026-10-18 Windows x64 初始集成记录（登录接入前）

| 检查 | 结果 | 证据范围 |
| --- | --- | --- |
| 完整 Rust 离线回归 | PASS，60 passed / 2 ignored | 两个 ignored 是已有 GitHub 在线测试，不计入通过数 |
| 最终进度补丁后的 tdl 分区 | PASS，11 passed | 增加每块 `flush` 等待 Tokio 后台写入；回调断言暂存文件实际长度等于上报字节数 |
| 活跃下载退出补强 | PASS，1 passed | 增强原队列测试，等实际写入字节后才 shutdown；相同测试定向重跑，无新测试数量 |
| Clippy | PASS | `--features desktop --all-targets -- -D warnings`，含最终补丁与测试 |
| 前端类型/构建 | PASS | `npm run build`；保留既有 use-client/sourcemap、包体积警告 |
| 浏览器交互 | PASS，mock IPC | 真实 App/组件，1440×900、1280×800、1024×680；单一原始规格、未知元数据、风险确认/拒绝、自定义路径保存、依赖错误及 ETA；不替代 Tauri 运行 |
| 内置引擎隔离启动 | PASS | `python scripts/check_packaged_engine.py python-engine/dist/video-engine/video-engine.exe`，0.7.0 / yt-dlp 2026.08.19，临时 cwd 且 PATH 无 Python |
| Windows x64 NSIS 构建 | PASS | 最终生产代码执行 `npm run tauri -- build --features desktop --config src-tauri/tauri.release.conf.json --bundles nsis -- --locked`；未安装、未发布 |
| 真实已登录 tdl / Telegram 在线访问 | NOT_RUN | 未获得专用测试环境及授权；依赖协议兼容证据是 pinned 上游源码核对与模拟服务 |
| 原生 Tauri GUI / Windows 安装运行 | NOT_RUN | 编译与安装包产物不等于实际安装运行 |
| macOS Intel / Apple Silicon 运行及安装包 | NOT_RUN | 本轮在 Windows 执行，不以其他平台结果替代 |

初始集成安装包为 26,262,717 字节，SHA-256 `5b3aa7e2afd536b0ebcaa7af20035c9e2dab0033c5233584e8e96deb193dc78e`；该本地产物已由下述登录版本替换。来自未提交工作区（包含先前 Markdown 功能），不表示已创建 Release。

### 2026-10-18 运行依赖登录接入与首轮验证（完成 2FA 前）

| 检查 | 结果 | 证据范围 |
| --- | --- | --- |
| 实际本机依赖检测 | PASS | `C:\Go\bin\tdl.exe`，v0.20.4，支持 `dl --serve/--port/--url`；版本探测隔离存储 |
| 用户授权消息默认会话解析 | FAIL | `https://t.me/caihongclub/55798`，官方返回 `not authorized. please login first`，未下载文件，未重写既有会话 |
| 完整 Rust 回归（登录基础接入） | PASS，64 passed / 2 ignored | 含原 HTTP/队列/提交及新增登录覆盖；两个 ignored 为既有 GitHub 在线测试 |
| 最终认证分区 | PASS，6 passed | `cargo test --locked --manifest-path src-tauri/Cargo.toml login -- --test-threads=1`；独立存储成功/失败/重启、取消/退出、特殊路径/退出码、真实 Windows 模拟终端子孙回收和错误停留、迟到 PID/缺失退出确认握手 |
| 最终 Clippy | PASS | `--features desktop --all-targets -- -D warnings` |
| 前端类型/构建 | PASS | 最终 NSIS 构建再次执行 `npm run build`；保留既有 use-client/sourcemap、包体积警告 |
| 登录组件交互 | PASS，mock IPC | 1440×900、1280×800、1024×680；取消、成功、重新登录失败保留状态、键盘 Enter、登录期间禁用配置、最终错误停留提示，无横向溢出 |
| 真实官方扫码终端启动 | PASS（仅启动） | 用户确认看见终端；独立测试 Core 调用生产 Rust 登录路径，不读取二维码或密码 |
| 真实扫码认证完成 | FAIL / 待用户继续 | 两次官方登录非成功退出，`TDL_LOGIN_FAILED`；未取得可判定根因的原始错误，用户表示稍后处理两步验证。不是账号失效判定。失败尝试目录均已清理，原配置保留 |
| 已登录后的真实解析/完整下载 | NOT_RUN | 尚未获得成功认证会话；不将模拟或终端启动记为真实下载成功 |
| 最终 Windows x64 NSIS 构建 | PASS | 同上 NSIS 命令，未安装或发布；生产代码含失败终端错误停留修复 |
| 原生 Tauri 页面 / Windows 安装运行 | NOT_RUN | 浏览器 mock 与真实官方终端不替代原生页面及安装运行 |
| macOS Intel / Apple Silicon | NOT_RUN | 取消握手模拟已测试；系统 Terminal、运行与打包仍须对应平台实际验证 |

当时的本地安装包为 26,305,938 字节，SHA-256 `fa53acec30054d74868048f0c44b3aa86c3d691cda1dea890fdaccf2da078318`，已由下述界面内认证版本替换。未自动安装、提交、推送或发布。

### 2026-10-18 完成 2FA 后的真实解析与完整下载

用户授权重新打开官方 QR 登录终端，并继续验证此前的 `https://t.me/caihongclub/55798`。使用独立测试 Core、SQLite 和新建认证目录，不修改用户原有 tdl 存储，也不改映取日常使用的配置。第 3 次尝试失败后，用户确认两步验证密码输错；第 4 次正确认证后，生产 Rust 登录、解析、下载、提交与重启恢复路径全部执行成功。认证输入仅在官方终端处理，未读取或记录二维码、验证码、密码或会话内容。

| 检查 | 结果 | 实际证据 |
| --- | --- | --- |
| 官方二维码与 2FA 登录 | PASS | `Core::login_tdl` 调用本机 tdl v0.20.4，官方成功退出后提交独立会话配置 |
| 真实解析 / HEAD | PASS | 频道 ID `1686995377`、消息 ID `55798`，组合身份 `1686995377_55798`；`video/mp4`，49,148,337 字节，唯一 `original` 规格 |
| 完整流式 GET 与进度 | PASS | 94 次非零下载进度，字节单调增长，最终接收/落盘/任务总字节均为 49,148,337；HTTP、类型、文件名及文件头经生产路径校验 |
| 原始文件提交与 SQLite | PASS | `completed`，任务 ID `6e0817e1-829a-4986-be93-6fa8fe3678a8`；现有安全提交记录与目标路径/大小相符，重开 Core 后记录恢复且文件存在 |
| 独立文件检查 | PASS | PowerShell 再次计算文件长度与 SHA-256，均与下载报告一致；SHA-256 `4caa8d1a792f2676dd4b89395bd3f762206ca93fd4db558cc35b74df3f4ec20e` |
| FFprobe 与完整解码 | PASS | MP4、H.264/AAC、720×960、554.72 秒；`ffmpeg -hide_banner -v error -xerror -i <file> -map 0:v:0 -map 0:a? -f null -` 退出 0，无解码错误。该检查不改写文件，也不回填解析阶段未知的规格字段 |
| 进程与暂存回收 | PASS | 本次测试的 tdl 进程无残留，`task_staging` 无记录，下载目录无 `.part` 文件；成功创建的独立登录存储保留供后续测试，既有会话不变 |
| 原生 Tauri 页面 / 安装运行 / macOS | NOT_RUN | 本次真实验证通过独立探针直接调用生产 Rust Core；不替代原生页面、安装包运行或其他平台验收 |

本机完整产物位于 `%LOCALAPPDATA%\Yingqu\tdl-validation\20261018-caihong-55798-login-r4\downloads\telegram_1686995377_55798_6e0817e1-829a-4986-be93-6fa8fe3678a8.mp4`，同级测试根目录 `result.json` 保存任务、文件大小和哈希。日志与独立检查证据为 `.local/tdl-live-login-r4.log`、`.local/tdl-live-r4-ffprobe.json`、`.local/tdl-live-r4-checks.json`。本轮没有生产代码变更，没有提交、推送或安装软件。

### 2026-10-18 界面内 QR / 2FA 验证

外部登录终端已替换为运行依赖页的 shadcn Dialog。二维码在弹窗内显示，官方要求两步验证后才出现密码框；成功后自动关闭弹窗并在卡片保留成功提示，错误保留供查看和重新扫码。验证窗口使用单独的 `me.disign.yingqu.tdl-ui-validation` 标识和数据目录，不切换映取日常配置，也不修改既有 tdl 登录存储。

| 检查 | 结果 | 实际证据 |
| --- | --- | --- |
| 完整 Rust 回归 | PASS，67 passed / 2 ignored | 两个 ignored 为既有 GitHub 在线测试；含隐藏 PTY、错误密码、取消及进程树回收 |
| 最终认证修复后定向测试 | PASS，`login` 6 passed、`hidden_pty` 2 passed | 修复 Unix 已回收根进程重复发信号的问题；Windows 回归通过，新增 Unix 专项未在本机执行 |
| Clippy / 前端构建 | PASS | desktop/all-targets `-D warnings`；最终 NSIS 构建执行类型检查和 Vite 构建，保留既有构建警告 |
| 浏览器登录组件 | PASS，mock IPC | 三个设计视口、QR→2FA、错误密码/重试、Enter 提交与清空、Escape/取消、组件重挂载及成功自动关闭；未使用真实密码 |
| Windows 原生界面 QR / 2FA | PASS | 用户在独立 Tauri 窗口完成扫码和密码输入；只读查询确认独立会话路径已保存且目录存在。用户反馈成功后弹窗停留，随后修复为自动关闭，并通过浏览器交互验证；未再次要求用户登录 |
| 新界面会话真实解析与完整下载 | PASS | 独立探针只读取得上述测试数据库的会话路径，在新建测试数据库中引用它，不复制认证数据、不使用旧验证会话。生产 Core 解析身份 `1686995377_55798`，原始 MP4 49,148,337 字节，94 次非零字节进度单调增长 |
| 文件与完成记录 | PASS | `completed`，重开 Core 恢复；独立长度/SHA-256、SQLite quick_check、暂存清理通过。SHA-256 `4caa8d1a792f2676dd4b89395bd3f762206ca93fd4db558cc35b74df3f4ec20e`，与上一轮相同；FFprobe 720×960、H.264/AAC、554.72 秒，FFmpeg 完整解码退出 0；无残留 tdl 进程 |
| Windows x64 NSIS 构建 / 许可资源 | PASS | 最终代码构建，打包目录中的 `licenses/tdl-ui-licenses.txt` 与仓库副本哈希相同；未安装或发布 |
| Windows 安装后运行 / macOS Intel、Apple Silicon | NOT_RUN | 原生开发运行、模拟测试和 NSIS 构建不替代安装运行或跨平台验收 |

本轮下载产物：`%LOCALAPPDATA%\Yingqu\tdl-validation\20261018-caihong-55798-gui-login\downloads\telegram_1686995377_55798_e9143b2f-62ae-42aa-98c9-54f1d6b24c0e.mp4`。证据：`.local/tdl-ui-live.log`、`.local/tdl-ui-live-checks.json`、`.local/tdl-ui-live-ffprobe.json`、`.local/tdl-ui-package-final.log`。仅保留成功的独立测试会话供后续验证，没有提交或推送。

界面内认证版本的安装包为 26,372,334 字节，SHA-256 `59469edfdbf8795d66626d2c51c0a3096a1b8891cb510858a628f8e3f8fd8e24`，已由下述设置分类调整版本替换。

### 2026-10-18 依赖与平台账号分类调整

- 「运行依赖」依次显示 FFmpeg、JavaScript 运行时、tdl；tdl 位于工具卡片之后、下载位置和内置引擎说明之前，只提供可执行路径配置与检测。FFmpeg/Deno 的缺失依赖下载保留。
- 「平台账号 → Telegram」统一提供登录/重新登录、扫码与 2FA、取消登录、HTTP 实验模式开关及风险确认。登录使用已保存路径；两处共用进行中状态，各自保存时保留另一处已保存字段及未保存草稿。
- PASS：最终前端类型/构建；完整 App 的 mock IPC 浏览器验证，覆盖入口与顺序、风险接受/拒绝、跨分类草稿、延迟保存互斥、路径与开关互不覆盖、使用已保存路径登录、QR/2FA/Enter/清空/成功自动关闭/Escape、四平台键盘标签、依赖下载、错误与三个设计视口。代码与验证脚本见本地交付记录；不是再次执行真实登录或下载。
- PASS：Windows x64 NSIS 构建，证据 `.local/tdl-settings-split-package-final.log`；浏览器证据 `.local/tdl-settings-split-browser.log`、`.local/tdl-settings-split-preserve-path.log`。本轮仅调整 React 组件、入口文案和文档，Rust/IPC 认证及下载逻辑未变，未操作真实登录会话、提交或推送。
- NOT_RUN：本次调整后的原生 Tauri 页面、Windows 安装后运行与 macOS Intel/Apple Silicon；前述真实认证/下载记录属于上一轮验证，不替代本轮未执行项目。

设置分类调整版本的安装包为 26,367,280 字节，SHA-256 `e2f97ad528c36d18b47c645011b01c4df54842f9cf486dfa637f85e494389c3c`，已由下述自动发现版本替换。

### 2026-10-18 自动发现与版本检测调整

- 软件取得已保存配置后自动查找 tdl，只执行隔离存储的 `version` 命令，不探测 serve/login，不绑定版本号；只返回路径及实际版本。结果和错误在本次运行跨页面保留，手动检查可重试，保存新路径后自动重新检查。版本命令异常退出或无法返回版本信息时不能报告可用。
- 自动发现覆盖 PATH、GOBIN、各 GOPATH/bin、Go 持久化配置和常见安装位置。Windows 查询注册表中的 PATH/GOBIN/GOPATH，并包含 `C:\Go\bin` 及 Program Files/Go/bin；macOS 保留现有工具路径发现。`go env` 限时读取 GOBIN/GOPATH，并显式使用本机工具链，不修改 Go 配置或下载工具链。
- PASS：tdl 分区 19 项测试；将既有登录和不同大小流式下载用例的模拟版本改为 9.1.0 后，两项定向测试通过。覆盖版本命令独占、异常退出、无效输出、不同版本号、运行时真实命令能力检查、多 GOPATH、路径空格、去重、持久化 go env 与本机工具链设置。Clippy desktop/all-targets `-D warnings` 通过。
- PASS：真实生产 `check(None)` 在本机未配置自定义路径的情况下自动发现 `C:\Go\bin\tdl.exe`，返回 0.20.4（`.local/tdl-discovery-live.log`）；本轮只检测可执行依赖，未登录、下载或操作既有会话。
- PASS：完整 App mock IPC 浏览器验证，启动自动检测一次、跨页面保留结果/错误、不重复检查、手动检查、保存路径重新检查、不将未检查的 serve 宣称可用、任意版本号展示、三个设计视口；证据 `.local/tdl-discovery-browser.log`、`.local/tdl-discovery-browser-check.js`。最终类型/前端构建与 Windows x64 NSIS 构建通过，日志 `.local/tdl-discovery-package.log`。
- NOT_RUN：本次调整的原生 Tauri 页面、安装后运行、macOS Intel/Apple Silicon；真实依赖检测不替代真实认证/下载或跨平台验收。未提交、推送或安装软件。

最新本地安装包：`src-tauri/target/release/bundle/nsis/映取_0.1.0_x64-setup.exe`，26,371,224 字节，SHA-256 `d4c2d7936447e0af2206bd386a61021620c87a453a04e4588c46226c00b22b70`。

### 2026-10-18 共享服务并发解析修复

| 检查 | 结果 | 证据与边界 |
| --- | --- | --- |
| tdl 模拟分区 | PASS，23 passed | 同服务下载中解析同频道/不同数字频道；新用户名立即失败且后续可继续解析；HEAD 取消/HTTP 错误不中断其他 GET；两条真实 Core 任务共享服务并独立取消/完成；缓存键碰撞拒绝；t.me / telegram.me 用户名改属后拒绝错下载、移除失效别名并恢复数字链接下载；重启、去重、提交与资源清理 |
| 原任务核心回归 | PASS，24 passed | `cargo test --locked --manifest-path src-tauri/Cargo.toml desktop_core -- --test-threads=1`，保留现有 Python 平台的队列、取消、格式选择、缓存、文件提交和恢复行为 |
| Rust 静态检查 | PASS | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` |
| 浏览器界面 mock | PASS | Playwright 驱动实际 App；已有下载时解析成功、未知用户名失败提示持续可见、按钮重新可用、随后数字链接可解析、键盘 Enter、三视口无横向溢出。失败且平台未知时不再显示“识别中”。不替代原生或在线验证 |
| Windows x64 调试构建 | PASS | `npm exec tauri -- build --debug --features desktop --config src-tauri/tauri.release.conf.json --no-bundle`，含前端类型/构建检查；保留原有 Vite 与 STATIC_VCRUNTIME 警告。不是安装包或原生运行验收 |
| 本轮真实 Telegram 并发 / 原生界面 / 安装运行 | NOT_RUN | 用户仍在运行原 release 应用及 tdl；未中断现有下载，未登录、复制或改写会话配置。此前真实下载记录不替代本轮新行为在线验证 |
| macOS Intel / Apple Silicon | NOT_RUN | 本轮仅在 Windows 运行模拟服务和构建 |

本轮 Windows 调试可执行文件为 `src-tauri/target/debug/yingqu.exe`，SHA-256 `4ca5d2511558e764fe1d22ac080e79add8b8f6aec65ed45337f4064cc9995722`；现有 release 可执行文件与安装包未替换，未自动提交或推送。

### 2026-10-18 下载后抽帧封面

| 检查 | 结果 | 证据与边界 |
| --- | --- | --- |
| Rust 核心回归 | PASS，76 passed / 2 ignored | `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1`；新增 Telegram 封面提交、读取、重启、缓存命中、删除视频后保留封面、失败不影响完成、抽帧期间取消与退出；两个 ignored 是已有 GitHub 在线测试 |
| Python 定向回归 | PASS，107 passed | `python -m pytest -q tests/test_local_cover.py tests/test_engine.py tests/test_runtime_dependencies.py tests/test_probe.py`；含非法路径/播放列表、无 FFmpeg、执行错误/超时、无效/超大封面、无视频轨、短视频真实 FFmpeg 抽帧及原文件哈希保持；保留 bridge 子孙进程与文件句柄取消回归 |
| 静态检查 | PASS | desktop/all-targets Clippy `-D warnings`，前端 TypeScript / Vite 构建；保留既有 Vite 和 STATIC_VCRUNTIME 警告 |
| Windows 打包 sidecar 本地真实媒体 | PASS | 内置引擎 0.7.1，在 PATH 无 Python 的临时目录执行新操作；读取此前授权下载的 49,148,337 字节 MP4 的副本，输出 720×960 JPEG、58,098 字节，FFmpeg 解码通过；原文件和副本 SHA-256 均保持 `4caa8d1a792f2676dd4b89395bd3f762206ca93fd4db558cc35b74df3f4ec20e`。未联网、未调用或读取登录会话；证据 `.local/telegram-cover-real-local/result.json` |
| Windows x64 调试应用 | PASS，构建 | 重建 Python sidecar 后执行 `npm exec tauri -- build --debug --features desktop --config src-tauri/tauri.release.conf.json --no-bundle`，未替换正在运行的 release 应用 |
| 本轮真实 Telegram 在线下载 / 原生 GUI 验收 / 安装包 | NOT_RUN | 用本地真实视频验证抽帧；任务集成使用独立模拟 HTTP 与协议 fixture。未中断用户正在运行的应用，没有重新登录、提交或推送 |
| macOS Intel / Apple Silicon | NOT_RUN | 延续跨平台实现，尚未在 macOS 运行抽帧或打包验证 |

### 2026-10-18 封面仅保存在 SQLite 及移除清理

- PASS：完整 Rust 核心回归 `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1`，78 passed / 2 既有 GitHub 在线测试 ignored。封面专用 4 项在补充“封面产生前已存在的预览也引用同身份缓存”边界后复测通过。覆盖下载目录仅有视频、原 IPC 字节读取、正常重启、提交后中断恢复、同作品重复记录/预览共享、不同频道隔离、最后引用移除清理、清理事务失败整体回滚、缓存写入失败仍完成视频；其他平台原封面保存测试保持通过。
- PASS：`tests/telegram-cover-cache.browser.js` 两项 mock IPC 浏览器验证，真实 App 在完成事件和恢复记录两种情况下仅凭 `cover_cached=true`、`cover_path=null` 读取本地图片字节、成功解码并显示；三个设计视口无溢出。证据 `.local/telegram-cover-sqlite-browser.log`。这不替代原生或在线验证。
- PASS：desktop/all-targets Clippy `-D warnings`；Windows `npm run desktop:build`，包含 TypeScript / Vite 与 release no-bundle 构建。Python 封面协议和引擎源码没有修改，保持 0.7.1。
- NOT_RUN：本轮真实 Telegram 在线下载、原生 GUI、安装验收及 macOS Intel / Apple Silicon。未中断现有验收窗口，没有操作、复制或修改登录会话，没有提交或推送。
