# 微信视频号本地解析与下载验证

2026-11-13，Windows 11（26200），Python 3.12.13，yt-dlp 2026.10.26。直接请求腾讯 HTTPS 接口，使用项目 `GuardedRH`，未配置应用代理。**P0：手动 Cookie 路线通过实施门槛。** 匿名未成功。默认浏览器读取是当时的探测路径，现已移除。

## P0 实测

| 实验 | 实际结果 |
| --- | --- |
| 匿名请求 | 5 个不同链接均返回 HTTP 401；JSON 有 `error.code` 和 `error.message`。这些样本未观察到匿名成功。 |
| 手动元宝 Cookie | 同一份本机 Cookie 发起 5 次，均返回 HTTP 200 / `code: 0`。3 条视频分享链接和 1 条图集链接有完整播放参数；合成无效短码返回空 `wx_export_id` 和 `playable_url`。 |
| 默认浏览器 Cookie | Windows 能识别默认 Edge。现有 yt-dlp 提取器报告 Cookie 数据库复制失败，未拿到 Cookie，未发送浏览器认证请求。未确认浏览器当前是否已登录元宝。 |
| 视频号详情 | 4 次均 HTTP **201**、`errCode: 0`、`data.errMsg.type: 0`。Cookie 未传给详情接口。3 次为 `mediaType: 4`，1 次为 `mediaType: 2`，图集无视频候选。 |
| 媒体传输 | 1 个视频的 2 个独立候选均从 `https://finder.video.qq.com` 直接 HTTP 200 完整落盘，无跳转。默认 `videoUrl` 与已列候选重复。CDN 不使用元宝 Cookie。 |
| H.264 | 5,112,296 字节，MP4，720×1280，H.264 + AAC；ffprobe 读出轨道，ffmpeg 完整解码通过。 |
| H.265 | 3,290,418 字节，MP4，720×1280，HEVC + AAC；ffprobe 读出轨道，ffmpeg 完整解码通过。 |
| 加密 | 2/2 独立候选无需解密即可解码。4 份有效详情均未观察到已检查的加密标志或 `decodeKey`；确认受保护样本 0 个。不能据此判断所有分享媒体均未加密。 |
| 作品标识 | 公开来源记录为同一作品的两个不同短链：`wx_export_id` 相等，`eid` 和 `sceneInfo.dynamicExportId` 不等。后续以带平台前缀的 `wx_export_id` 哈希为身份，不持久化原值；结论限于本次样本。 |
| 其他环境和异常 | P0 未验证 macOS、Edge 解锁后的浏览器 Cookie、真实失效 Cookie、真实限流和代理环境。后续代理和无效 Cookie 验证见 P4；尚未覆盖方案的 20 条授权样本矩阵。 |

请求结构（占位值）：

```text
POST https://yuanbao.tencent.com/api/weixin/get_parse_result
{"type":"video_channel_url","url":"https://weixin.qq.com/sph/<share>","scene":1}

POST https://channels.weixin.qq.com/finder-preview/api/feed/get_feed_info?_rid=<request>&_pageUrl=<feed-page>
{"baseReq":{"generalToken":"<token>"},"exportId":"<eid>"}
```

最小请求使用标准 JSON、Origin、Referer、User-Agent 头；未使用固定个人 ID、设备 ID、网页版本号或公共解析网关。元宝 `data.playable_url` 仅用于解析 `eid` 和 `token`，不会请求它。详情返回 `data.authorInfo.nickname`、`data.feedInfo.description`、`coverUrl`、`createtime`、`videoUrl`、`h264VideoInfo.videoUrl` 和 `h265VideoInfo.videoUrl`。未观察到时长或规格分辨率字段；媒体探测所得尺寸不能用于其它作品。

复现（在 PowerShell 使用有权下载的样本；Cookie 文件仅留本机）：

```powershell
.venv/Scripts/python.exe scripts/probe_weixin_channels.py <分享链接> --cookie-file tests/yuanbao.txt --download
.venv/Scripts/python.exe scripts/probe_weixin_channels.py <同作品链接1> <同作品链接2> --cookie-file tests/yuanbao.txt --compare-first-two
```

样本来源：[公开解析示例](https://github.com/ucmao/media-parser/blob/main/docs/parsers/wechat-channels.md)、[同作品多次分享案例](https://www.lovstudio.ai/cases/wechat-channels-restricted-long-video-client-capture)。样本媒体在临时目录完整下载，探测后删除。网络回归 `pytest -q tests/test_network.py`：12 通过。

## P1–P4 验证

| 阶段 | 证据与实际结果 |
| --- | --- |
| P1 解析 | `pytest -q tests/test_weixin_channels.py tests/test_network.py`：阶段检查 30 通过。验证匿名成功不读 Cookie、明确 401 才重试、手动 Cookie 优先、浏览器 Cookie 作用域、无效链接与结构变化分离、CDN 白名单、受保护候选拒绝。 |
| P2 引擎 | Windows `m1-probe` 连真实接口，经 Rust/Python JSON Lines、yt-dlp、FFprobe 和 Rust 文件提交，H.264 生成 5,112,296 字节 MP4。定向用例验证身份不匹配、规格消失时不启动媒体传输，以及缓存别名保留当前短链。 |
| P3 前端 | `npm run build`（TypeScript + Vite）通过。Playwright 模拟 IPC 测试 `tests/weixin-channels.browser.js` 在 1440×900、1280×800、1024×680 全通过，覆盖键盘解析、登录入口、Cookie 保存、重试、H.265 选择与平台筛选。 |
| P4 离线回归 | Python 全量 544 通过，包含抖音视频/图集、TikTok、YouTube、小红书及桥接回归；最后调整媒体校验后，相关 11 项复跑通过。首次并行构建时取消用例未在 5 秒内收到代理请求，定向复跑及后续全量通过。Rust 全量 577 通过、31 个跳过；修复重启预览后，Rust lib 全量 558 通过、29 个跳过。包含 Telegram 离线队列、重试和取消。桌面 Clippy（`-D warnings`）通过。 |
| P4 Windows 真实接口 | 无手动 Cookie 时桥接返回 `BROWSER_COOKIES_UNAVAILABLE`；合成无效 Cookie 经腾讯真实接口拒绝后返回 `WEIXIN_COOKIE_EXPIRED`；现有有效 Cookie 通过。使用本地 HTTP CONNECT 代理，三个目标都经过代理；在 CDN 建连时取消，Rust 退出子进程并清理 staging，无完成文件；随后经同一代理重试下载成功。直连和代理均有真实证据。 |
| P4 Windows 原生及打包引擎 | 隔离的 Windows Tauri 实例使用 PyInstaller 引擎，真实解析、封面、H.265 选择、立即下载、任务历史及最终文件通过；产物 3,290,418 字节，FFprobe 读到 HEVC 720×1280 + AAC。改为完整解码后重打包，版本握手通过（协议 v3、引擎 0.15.0），用最终打包引擎经 Rust 探针重新下载 H.264 成功。 |
| 独立复审 | 修复浏览器日志初始化、重启时覆盖当前短链、只抽查部分视频等问题后复审通过；多视频轨文件的第二轨后段损坏且字节数不变时也会拒绝提交。 |

解析只公开作品摘要与稳定规格。默认浏览器提取复用 yt-dlp 的读取器，读取后按目标域、路径及有效期筛选，只有适用 Cookie 用于元宝请求。下载时重新校验作品和所选规格；FFprobe 校验 MP4 和编码，FFmpeg 完整解码视频及已有音轨后才提交。未安装工具时，从「系统设置 → 运行依赖」安装 FFmpeg 工具包。媒体保护未得到可验证密钥与样本，不引入解密实现。客户端代理不修改系统设置。

**发布判定：可供 Windows 手动 Cookie 路线试用，跨平台正式发布门槛未通过。** 登录且可提取的默认浏览器、macOS Intel / Apple Silicon、20 条授权样本、真实限流及其他平台在线回归仍为 `NOT_RUN`；Windows 安装包未制作。原生界面取消/重试与代理开关未单独验收；上述成功的取消、重试和代理验证发生在 Windows Rust 探针中。

预览补充验证（引擎 0.15.1）：分享详情中的 H.264 和 H.265 字段各只含播放地址。分别请求 CDN 的前 128 KiB，均返回 HTTP 206，FFprobe 读到两种规格都是 720×1280。探测无结果时保留未知，下载继续使用同一规格 ID。详情的 `headImgUrl` 和元宝的 `author_icon` 在实测中均指向 `wx.qlogo.cn`，头像请求得到 HTTP 200 和 22,747 字节的 JPEG，预览复用本地头像缓存。

时长与大小补充验证（引擎 0.15.2）：复用预览的 HTTP Range 请求。文件大小来自通过校验的 HTTP 206 `Content-Range` 总字节数，H.264 为 5,112,296 字节，H.265 为 3,290,418 字节，与此前完整下载结果一致。FFprobe 从 MP4 头部的视频轨道读到两档均为 126.583 秒。缺少 FFprobe 时仍可读取大小，缺失的字段保留未知。

长视频补充验证（引擎 0.15.3）：新增样本两档的前置 MP4 元数据分别在约 3.59 MB 和 3.92 MB 处结束，均超过原有 128 KiB 探测范围。按头部标出的边界补读剩余元数据后，实测两档均为 1024×576、5628.746 秒。H.264 总大小为 252,523,576 字节，H.265 为 184,237,953 字节。探测范围最多 8 MiB，超限时保留已从 HTTP 响应确认的大小。此前短视频样本的预览结果也已复验。

默认浏览器复测（Windows 11，系统默认 Edge）：Edge 运行时返回 `COOKIE_DATABASE_COPY_FAILED`，Windows 文件读取确认共享冲突。数据库解锁后，使用同一条视频分享链接运行 `scripts/probe_weixin_channels.py <分享链接> --browser`，返回 `COOKIE_DECRYPT_FAILED`。只读检查发现 `yuanbao.tencent.com` 与 `tencent.com` 合计 12 条相关 Cookie 均使用 `v20` 加密，未输出 Cookie 名称或内容。现有读取器未获得可用 Cookie，因此未发送浏览器认证请求；同次匿名请求返回 HTTP 401。[Microsoft Edge 应用绑定加密说明](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-browser-policies/applicationboundencryptionenabled)

应用内登录可行性验证（Windows 11，Tauri 2.12.3 / WebView2）：独立数据目录的临时窗口成功加载元宝官网及其微信登录页面。另用本地 HTTP 测试服务设置合成 `HttpOnly` Cookie，Rust 从后台线程调用 `cookies_for_url`，读取验证通过。真实元宝登录、使用登录会话解析视频号，以及 macOS 的登录与 Cookie 读取均为 `NOT_RUN`。本次为独立验证，未接入应用的生产流程。

三种认证方式（Windows，引擎 0.15.4）：Playwright 在 1440×900、1280×800、1024×680 验证面板切换、激活、保存和复用凭据。Rust 定向回归验证旧配置迁移、凭据持久化及下载提交时更新认证方式。Python 引擎、视频号、探针和媒体回归 145 项通过，打包引擎握手通过。Windows 隔离预览版编译启动，旧预览数据中的元宝会话迁移后仍为生效方式。此次未发起真实账号登录或在线下载，macOS 原生验证为 `NOT_RUN`。

## 平台登录复测（Windows，引擎 0.15.5）

已移除系统浏览器 Cookie 自动导入。旧版选择默认浏览器的配置升级后进入匿名访问，已保存的两类凭据保留。手动 Cookie 与内置会话独立配置，解析只使用当前生效的凭据。微信视频号仍先尝试匿名，收到明确认证要求后才使用该凭据。

| 平台 | 官方页面实测 | Windows 原生会话验证 |
| --- | --- | --- |
| 抖音 | 独立浏览器一次遇到验证码中间页；原生窗口打开官方登录，页面显示扫码入口。 | 合成 HttpOnly 会话保存、自动关窗、清除通过。 |
| 小红书 | 独立浏览器一次出现网络风险 300012；最终版原生窗口显示扫码和手机号登录。 | 游客态 `web_session` 不保存；合成会话同时有 `web_session` 和 `id_token` 时保存、自动关窗、清除通过。 |
| TikTok | 原生窗口加载官方二维码及图片，关闭窗口仍为匿名。谷歌登录账号可先在手机端登录 TikTok，再从手机确认网页扫码。 | 合成 HttpOnly 会话保存、自动关窗、清除通过。 |
| YouTube | 原生按钮打开系统浏览器，官方入口跳转到 Google 登录。认证解析支持手动 Cookie。 | 安全行为由离线测试覆盖，包括匿名和使用显式提供的 Cookie。[Google OAuth 2.0 政策](https://developers.google.com/identity/protocols/oauth2/policies) |
| 微信视频号 | 原生窗口打开元宝官方登录并显示微信入口。 | 合成三项 HttpOnly 凭据保存、自动关窗、清除通过。 |

本轮未完成真实账号的扫码、短信或 Google 授权流程。登录后使用真实会话解析和下载、macOS 实机均为 `NOT_RUN`。合成测试在隔离数据目录验证应用的会话读取和保存，凭据注入前暂停小窗网络。

自动化结果：Python 全量 542 项通过。Rust 560 个库测试、7 个桌面测试、14 个集成测试通过；31 项按配置跳过。前端类型检查和构建、13 项平台账号及视频号浏览器用例、PyInstaller 引擎握手和独立代码审查通过。额外运行的全站辅助文字用例通过账号部分，随后在关于页的更新说明正文等待超时。
