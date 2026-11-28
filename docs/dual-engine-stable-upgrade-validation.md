# 双引擎稳定版升级验证

2026-11-28，本次仅升级、验证并启动 Windows 本地预览，不提交、推送或发布。保留升级前已有的 Telegram 限流提示修复。

## 固定版本与适配

| 组件 | 本次版本 |
| --- | --- |
| yt-dlp | `2026.11.23` |
| yt-dlp-ejs | `0.8.0`，符合新版本的精确依赖要求 |
| DTK | `5.1.13`，提交 `50f89e8ba23c205e06bfeed78246daf81583276b` |
| Python / 引擎 / 协议 | `3.12.13` / `0.17.0` / `3` |

来源为 [yt-dlp 稳定发行](https://github.com/yt-dlp/yt-dlp/releases/tag/2026.11.23)与 [DTK 稳定发行](https://github.com/Evil0ctal/Douyin_TikTok_Download_API/releases/tag/v5.1.13)。锁文件只更新 yt-dlp；EJS、证书及其他依赖保持原版本。yt-dlp wheel/sdist 哈希见[上游说明](../vendor/UPSTREAM.md)，已安装 wheel 的许可证与现存分发文本一致。

DTK 由既有同步脚本从完整提交生成，更新原始/安装后文件哈希、revision 与导入隔离补丁，不修改上游签名和 parser 算法，不引入服务端或 browser-rpc。

TikTok 详情请求先通过现有 GuardedRH 读取同一会话主页，复用上游设备 ID parser；使用主页刷新后的 CookieJar 签名，设备 ID 缺失或页面超限时明确失败。session-bound 视频仅将当前会话匿名 `tt_chain_token` 写成 secure `.tiktok.com` Cookie；登录 Cookie 保持 host-only，token 不发送给 tiktokcdn/tiktokv 或 HTTP。媒体 Referer 使用上游 `MediaAccess`；`MEDIA_WITHHELD` 映射为既有 `UNSUPPORTED_CONTENT`。

## 当前验证

- PASS：`update_dtk_vendor.py --check`，源码、补丁哈希及导入边界。
- PASS：`uv pip check`；Python 全量 `pytest -q --tb=short`，586 项通过。
- PASS：DTK、浏览器身份和切片同步定向回归，54 项通过。覆盖真实设备 ID、刷新 token、签名参数、异常页面、媒体域 Cookie 隔离、只读模型、Referer 和媒体不可提供错误。
- PASS：Rust probe 构建、`cargo clippy --features desktop,probe --all-targets -- -D warnings`、前端类型检查及生产构建。
- PASS：Rust 全量测试，合计 613 项通过、33 项按原配置忽略（主要为需显式授权的在线用例），无失败。
- PASS：PyInstaller 内置引擎重建；隔离 Python 环境变量和 PATH、使用临时工作目录启动完整 onedir，协议 `3`、引擎 `0.17.0`、yt-dlp `2026.11.23`、DTK `5.1.13` 及完整提交与当前契约一致。
- PASS：`npm run desktop:build -- --debug -- --locked`。程序位于 `src-tauri/target/debug/yingqu.exe`，已按完整路径启动并确认进程来自该目录；原生窗口标题为“映取”，主界面“下载作品”、输入框、解析引擎和侧栏正常载入，进程响应正常，并保留窗口供用户查看。侧栏 `0.1.0` 是本地 debug 构建的应用版本，不是新的正式发行版本。

本机脱敏日志位于忽略目录 `.local/dual-engine-upgrade-20261128/`。首轮新增测试夹具和 Douyin profile 参数回归已修复，并通过最终全量 Python 测试；未弱化原有断言。

## 验收边界

本轮网络回归使用隔离 fixture；真实平台登录、有效公开视频/图集下载、macOS Intel/Apple Silicon、安装包及无 Python 干净环境验收为 `NOT_RUN`。既有平台成功记录不作为新版本在线验收。
