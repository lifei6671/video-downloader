# B 站视频下载

使用现有固定版本 yt-dlp `2026.10.26` 的 `BiliBili` 提取器，不新增抓取 SDK 或下载器。当前引擎版本为 `0.17.0`，Python/Rust 握手版本一起更新，协议仍为 v3；旧打包引擎会被版本检查拒绝，须随桌面程序重建。

## 输入与下载

- 支持 `bilibili.com/video/BV...`、移动端对应链接、`b23.tv` 短链以及带单个链接的分享文案。
- 只处理选定视频的一个分 P，默认 P1；保留 `p` 参数，去掉分享跟踪参数。`BV..._pN` 是内部稳定身份，防止不同分 P 共用下载记录、缓存或备份。不会展开合集或播放列表；番剧、直播、动态、用户主页及 AV 号链接不在本轮范围内。
- 复用现有清晰度/编码选择、音视频分离流配对、下载进度、取消、FFmpeg 无转码合并及 Rust 最终文件提交。仅列出当前会话实际可获取的规格，重新提取时坚持所选规格，不静默降低清晰度。
- 默认沿用现有分辨率、帧率和码率排序选择规格，用户可切换编码。分离流需要本机 FFmpeg；缺少时使用现有 `FFMPEG_UNAVAILABLE` 失败机制。
- B 站图片 CDN 的 HTTP 封面地址只在已允许的官方图片域中转换为 HTTPS，复用原有封面校验和缓存。
- 新增 B 站平台账号和任务筛选。根据视频返回的 UID 请求官方作者卡片，核对 UID 后读取头像与个人简介；头像经过现有校验写入 SQLite，远程地址不进入任务记录。预览、任务列表和作者主页共用作者资料。旧 B 站解析缓存会在下次解析时重新获取，补齐同一作者的现有记录；不在启动时批量请求历史作者。资料接口失败、返回错误身份或图片不可用时不影响视频解析与下载。
- 作者表的平台约束包含 `bilibili`。已有作者表在启动事务中自动迁移，迁移前保留数据库快照，完整保留作者资料、头像及任务关联；不增加 UP 主批量抓取。
- 已完成 B 站视频沿用现有 S3/R2、Telegram 备份与按需恢复，来源 URL 保留分 P 参数。

网络请求继续由现有受限传输层负责，只开放 B 站站点、短链、媒体和图片 CDN。官方 `*.mcdn.bilivideo.cn` 媒体节点支持 HTTPS 8082 端口，其余目标仍沿用原有端口限制。Cookies 发送到 B 站主站及官方 API，不随跨站 CDN 请求或其他平台请求发送。

## Cookie 与登录

入口：系统设置 → 平台账号 → B 站。登录与下载的浏览器身份绑定及站点存储规则见[设备信息](platform-device-identity.md)。

手动 Cookie 和内置登录沿用既有两套独立存储及来源切换。手动 Cookie 可保存、读取、清空；内置登录打开 `passport.bilibili.com` 官方登录页，由用户自行登录。发现适用于主站、未过期的 `SESSDATA` 后，自动保存会话、选择内置登录来源并关闭窗口；取消或超时不覆盖原有凭据。登录窗口只允许主站及官方登录域导航。

会话沿用项目现有 Cookie 存储契约及任务配置快照，不打印、不放入命令行或测试样本。后续新任务使用选定凭据，已捕获配置的任务保持原快照。B 站官方 API 与网页主站需要共享父域 Cookie，因此内置登录 Cookie 在引擎中限定为 `.bilibili.com`；不套用其他平台的 `www` 主机限定。

登录不会赋予账号本身没有的内容或清晰度权限；界面只显示上游实际返回的可下载规格。

## 验证入口与边界

- Python：`.venv/Scripts/python.exe -m pytest -q tests/test_bilibili.py tests/test_youtube.py tests/test_network.py`。
- Rust：设置 `CARGO_PROFILE_TEST_DEBUG=0` 后，执行 `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib bilibili -- --test-threads=1`；以及 `cargo test --locked --manifest-path src-tauri/Cargo.toml --features desktop --bin yingqu platform_login -- --test-threads=1`。
- 浏览器：开发服务器下打开 `tests/fixtures/platform-settings.html`，使用 `playwright-cli --session bilibili run-code --filename tests/bilibili-settings.browser.js`，覆盖三种规定视口、手动 Cookie、登录结果、来源切换、重新加载及任务筛选；凭据和 IPC 为模拟数据。
- 打包：使用现有 PyInstaller spec 重建，并执行 `scripts/check_packaged_engine.py`；必须通过隔离 cwd/PATH 启动校验。
- 桌面核心在线回归：设置 `YINGQU_BILIBILI_TEST_URL` 为待验证视频，执行 `cargo test --locked --manifest-path src-tauri/Cargo.toml --test bilibili_download_live -- --ignored --nocapture`。使用隔离数据库和打包引擎匿名验证解析、下载、作者入库及重启恢复，不读取用户凭据。

2026-11-22 使用用户提供的 `https://www.bilibili.com/video/BV13XH56SEYt/` 完成 Windows 源码引擎和 PyInstaller 引擎的真实匿名下载。两者均经 Rust `m1-probe` 的协议、媒体提交与不覆盖检查，输出 14,362,733 字节 MP4。FFprobe 识别为 1920×1080 H.264 视频与 AAC 音轨，时长 73.966625 秒；完整 FFmpeg 解码通过。测试文件在忽略目录 `.local/bilibili/`，未读用户真实 Cookie。

同日使用 `BV1mMWu6wEwH` 的完整分享链接，经隔离数据库、桌面 `Core` 和 `0.16.1` 打包引擎完成真实匿名解析及下载，输出 43,384,267 字节。作者与任务成功入库，关闭并重新打开核心后，任务保持完成状态，作者资料与媒体字节完全一致。离线回归同时覆盖新库、两种旧作者表的保留迁移、迁移前快照、失败回滚及外键完整性。

真实账号扫码/密码登录后自动保存的原生端到端、macOS Intel、macOS Apple Silicon、安装包验收：`NOT_RUN`。Cookie 识别和持久化由 Rust 单元测试与浏览器模拟验证，不能代替上述真实账号验收。

作者资料补充在 `0.16.2` 打包引擎中使用同一视频匿名验证：预览可读取简介和头像，下载后重启核心，头像字节和作者资料保持一致。浏览器模拟 IPC 验证三个规定视口下的预览、任务列表和作者主页，以及作者主页的键盘进入和焦点。
