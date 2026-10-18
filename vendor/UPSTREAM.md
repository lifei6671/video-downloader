# 解析引擎来源与依赖

当前使用 yt-dlp + DTK 双解析引擎，共用 yt-dlp 网络与 HTTP 下载器。默认视频由 yt-dlp 解析、图集由 DTK 解析，用户也可明确选择引擎；失败不会自动切换。官方 Python 包 `yt-dlp==2026.8.19` 的运行时版本为 `2026.08.19`；协议 v3，引擎版本 `0.6.0`。

- 来源：https://github.com/yt-dlp/yt-dlp/tree/2026.08.19
- PyPI：https://pypi.org/project/yt-dlp/2026.8.19/
- 官方 wheel：`yt_dlp-2026.8.19-py3-none-any.whl`
- wheel SHA-256：`1d57897e94c6665a0a6f9bc54b34e584284e32c034ffab3a7df25d8f7b24eedf`（PyPI 发布元数据核对）
- sdist SHA-256：`9e213e48cea35c66b378e4447903f118f6392a5fa380a2b6d7070ec86f4e0af1`
- Unlicense 原文保留于 `yt-dlp-LICENSE`，从已安装的对应 wheel 许可证复制。

不修改已安装的 yt-dlp 包；经用户明确批准，在自有适配层覆盖 DouyinIE 的单一详情请求构造，补丁范围见下文。`python-engine/engine.py` 使用 `YoutubeDL` 嵌入接口；`network.py` 为本应用的双平台网络边界适配，固定依赖该版本的 `UrllibRH`、`HTTPHandler` 和请求 director。沿用上游 Cookie、HTTP 解码、重定向及 SOCKS 实现，只增加逐跳目标校验与直连 DNS 结果固定；按 2026-10-14 用户 TUN 决策，不检查 DNS 返回 IP 的公网属性，私有/保留地址交由系统路由；升级须重测这些内部接入点。显式 HTTP 代理由适配层设置路由，避免环境 NO_PROXY 或系统 bypass 改走直连；SOCKS 保留上游实现。远端 DNS 代理的实际目的 IP 无法由本客户端验证。

YouTube 额外锁定 `yt-dlp-ejs==0.8.0`（yt-dlp 此版本的匹配版本），许可证 `Unlicense AND MIT AND ISC` 随 onedir 收集；使用官方 hook 收集离线 solver JS，禁用 remote components，不运行时下载脚本。Node.js / Deno 与 FFmpeg 由本机提供，不随应用分发。参考 [yt-dlp EJS 官方说明](https://github.com/yt-dlp/yt-dlp/wiki/EJS)。默认浏览器 Cookie 思路参考 [downvid](https://github.com/yxxbc/downvid/tree/b1ef96800f1bb999bf63bee6b9a0ad98996494ea/electron/utils)，本项目自行实现系统默认浏览器识别，未复制其多浏览器重试或 Puppeteer 服务。

运行锁文件新增 pydantic 2.14.1、structlog 26.1.0、httpx 0.28.1 与必要传递依赖，不启用 yt-dlp extras。httpx 是上游 token 模块的导入依赖，业务请求通过 GuardedRH；不调用其游客身份网络功能。structlog 自由文本在 bridge 中全部丢弃，避免 stdout 协议污染及凭据泄漏。没有 wreq、FastAPI、Redis、SQLAlchemy、上游 UI 或服务端。

PyInstaller 使用官方 yt-dlp hook，收集 DTK 的具体平台模块、完整 onedir 及实际运行依赖的许可证；DTK Apache LICENSE / NOTICE 一同分发。运行时不加载上游 RPC、签名 registry、身份池或后台服务。开发锁文件另有 pytest、PyInstaller，含 Windows/macOS 平台标记。旧 M1 引擎已退役，新切片不是恢复旧服务架构。

## DTK 固定切片与补丁

- 来源：[Evil0ctal/Douyin_TikTok_Download_API](https://github.com/Evil0ctal/Douyin_TikTok_Download_API/tree/4f0bed8483c35a980315d9c7b3a1d4a1119ad2b2)，版本 5.1.3，提交 `4f0bed8483c35a980315d9c7b3a1d4a1119ad2b2`。
- [manifest](dtk/manifest.json) 保存 31 个 Python 文件及 LICENSE / NOTICE 的原始、安装后 SHA-256，补丁 SHA-256 和允许的运行依赖。
- [desktop-imports.patch](dtk/patches/desktop-imports.patch) 仅隔离 signing 包初始化的 registry / RPC 聚合导入；签名算法、参数构造和 parser 保留上游原文。自有业务编排位于 `python-engine/dtk_adapter.py`。
- `vendor/dtk/upstream/` 由 [同步脚本](../scripts/update_dtk_vendor.py) 生成，不手改。`_desktop_revision.py` 是脚本生成的版本文件。
- 更新先校验已安装源文件，再从指定完整 commit 读取白名单、校验原始 hash、执行补丁 apply --check、审计导入与语法；全部通过才替换源码和 manifest。冲突、未审核依赖或本地修改不会被覆盖。
- 更新命令、补丁升级及门禁见 [开发文档](../docs/development.md#解析模块同步与补丁升级)。CI 和 Release 构建均离线执行 `--check`，不在用户运行时拉取模块。

正式图集 GUI、文件格式、重启及对应安装包验证见 [双引擎验收](../docs/dual-engine-validation.md)；初始独立试验与设计见 [双引擎方案](../docs/dtk-dual-engine-plan.md)。旧单引擎 Windows/macOS 证据不能替代此次双引擎对应平台验收。

## Douyin 详情请求补丁

2026-10-13 用户明确允许最小签名补丁，作为项目“不重写签名算法”约束的单一例外。`python-engine/douyin.py` 继承固定版本 DouyinIE，仅为 `https://www.douyin.com/aweme/v1/web/aweme/detail/` 添加请求参数，不替换其 `_real_extract`、媒体 parser、格式选择或下载器。

- 问题背景：[yt-dlp #9667](https://github.com/yt-dlp/yt-dlp/issues/9667)。本机原始与 guarded transport 对照均获得 `ArgusSecurityPlugin Uifid Not Found`，加入 Cookie 中 UIFID 后变为 `Signature Not Found`，加入正确签名后返回 200/status_code=0。
- 公式来源：[Evil0ctal 原始研究提交说明](https://github.com/Evil0ctal/Douyin_TikTok_Download_API/commit/d9f718800df20c0255979d93ebadb33dd3a3b757)。仅依据公布公式独立实现，没有复制 dtk 源文件、签名模块或恢复其依赖；使用 Python 标准库，不新增第三方包。
- 读取授权 Cookie 的 UIFID（或 UIFID_TEMP）；不生成游客身份、不调用浏览器或远程签名服务。请求包括 aweme_id、uifid；timestamp 不发送，签名中的 timestamp 字段为空。
- 签名为 `MD5(uifid + "_" + timestamp + "_" + salt + "_" + URLSearchParams(query))`；固定 salt 为该研究公布的 `A96D855A08C0A9707F8BEF0D9A527E4E`。编码按浏览器规则处理空格、星号、波浪号，测试向量由 Node URLSearchParams/crypto 独立生成。
- 未持有 UIFID 时请求前报错；Cookie、签名 query 和媒体 URL 不记录日志、不持久化。域名 `365yg.com` 来自本次签名成功详情返回的抖音视频 CDN，只加入抖音清单；每跳域名/协议/端口校验仍生效，直连仅解析一次，不按 DNS 返回 IP 的公网属性拒绝 TUN 路由。
- 上游加入可验证修复后优先移除此覆盖。固定版本升级或平台 salt/参数变化时，重新运行补丁测试、双平台实际下载、解码和打包验证；不自动切回旧引擎，不声称补丁永久有效。

## M2 桌面界面

生产界面使用 shadcn/ui 官方 radix-nova 注册表生成的组件，来源 https://ui.shadcn.com/r/styles/radix-nova/ ，CLI 4.21.0；组件保存在 `src/components/ui/`，MIT 许可副本见 `shadcn-LICENSE`（来自安装包 LICENSE.md）。仅 Progress 透传 value 到 Radix Root，以提供真实 ARIA 进度。页面仅组合这些组件与业务数据，不另建基础控件。React、Radix、Tailwind、Lucide 和 Tauri 的实际版本/完整性哈希由 package-lock.json 与 Cargo.lock 固定；没有运行时 CDN。


## 视频规格与抖音高清补充（2026-10-14）

用户批准在预览选择真实下载规格。`python-engine/video_formats.py` 只列单文件 MP4/WebM，移除 CDN 镜像重复项、水印、DRM 和不可播放 bytevc2；默认按实际像素数、帧率、码率排序，稳定规格 ID 不包含 URL 或签名。下载重新提取并严格匹配规格，缺失时 `FORMAT_UNAVAILABLE`，不自动降级或转码。

`python-engine/douyin.py` 的详情请求补充 `aid=6383`、`channel=channel_pc_web`、`detail_list=1`，仍使用此前签名公式及 GuardedRH。来源核对：[DownVid 抖音解析器](https://github.com/yxxbc/downvid/blob/main/electron/parsers/douyin.ts)。不引入其浏览器、UI 或网络实现。实际作品 `7696335899718697330` 原查询仅返回 720p；完整查询返回含 6290708 字节 4K 的播放规格。

yt-dlp 2026.08.19 的 URL-key parser 对该作品 `bytevc1_4k` 未填尺寸；自有 Douyin 子类在复用上游解析后，将同一 URL 对应 `play_addr` 的 width/height 填回各格式，不使用父级上传尺寸猜测。上游升级时检查完整请求参数、4K 格式尺寸和排序；上游能够正确完成这些行为后移除此补充，保留回归测试。签名算法没有改变。

4K 媒体请求显式使用平台根 Referer `https://www.douyin.com/`；本机同一规格 CDN 请求缺少该头时返回 403，带根 Referer 时返回 206。该头只用于抖音下载，TikTok 和 Cookie 域隔离保持不变。

## Telegram tdl 外部工具（2026-10-18）

按用户决定只调用用户自行安装的 [iyear/tdl v0.20.4](https://github.com/iyear/tdl/tree/9d7d49eef2bcb04da720c26e33598c49c68b9ddd)，commit `9d7d49eef2bcb04da720c26e33598c49c68b9ddd`，上游许可 AGPL-3.0。未 vendoring、复制其 Go 源码或随安装包分发 tdl；自有 Rust 适配器依据 `app/dl/serve.go`、`serve.go.tmpl`、`cmd/dl.go`、`cmd/version.go`、`cmd/root.go` 和 `core/util/tutil/tutil.go` 的 CLI/HTTP 契约实现。用户另行授权运行依赖登录，依据 `cmd/login.go`、`app/login/login.go`、`app/login/qr.go` 调用显式 QR 模式和独立存储；禁止默认 desktop 导入及覆盖既有命名空间。当前全接口监听、缺少令牌及请求白名单须作为已知实验风险持续呈现；升级和真实运行边界见 [集成说明](../docs/tdl-integration.md)。

### tdl 界面内认证桥接（2026-10-18）

仍执行用户安装的 tdl v0.20.4 `login --type qr`，不修改其 Go 代码。上游没有 JSON 认证接口；隐藏伪终端复用 `app/login/qr.go` 的二维码/2FA 流程，二维码像素还原依据 `skip2/go-qrcode` commit `da1b6568686e` 的 `ToSmallString(false)`，未复制其实现源码或引入 Telegram 认证库。

新增 Rust 直接依赖：`portable-pty 0.9.0`（MIT）负责 Windows ConPTY / Unix PTY，`vt100 0.16.2`（MIT）处理终端重绘及屏幕状态，`zeroize 1.9.1`（Apache-2.0 OR MIT）清理 Rust 临时密码内存。完整版本/校验值见 Cargo.lock；新增及直接使用依赖的许可文本见 `vendor/tdl-ui-licenses.txt`，随安装包放入 `licenses/tdl-ui-licenses.txt`。不新增 shell 登录脚本，不转发终端原文、二维码 URL 或密码到日志/持久化。

## 运行依赖下载（2026-10-18）

用户明确授权运行依赖页下载缺失工具，替代此前“只检测、不下载”的约束。仅作为应用专属外部工具，不更新内置解析引擎。

- FFmpeg/FFprobe：采用 [eugeneware/ffmpeg-static b6.1.1](https://github.com/eugeneware/ffmpeg-static/releases/tag/b6.1.1) 的 Windows x64 / macOS x64、arm64 gzip 静态构建。SHA-256、压缩大小直接核对该 Release 的公开资产元数据，与 Downvid 固定 manifest 对照一致；没有复用其源码。下载保留各平台 LICENSE、README（包含构建/源码说明），二者亦校验 SHA-256。许可依实际构建的 LGPL/GPL 声明，不将静态构建统一描述为 MIT。资产来源及校验值见 `python-engine/runtime_assets.py`。
- Deno：采用官方 [denoland/deno v2.9.7](https://github.com/denoland/deno/releases/tag/v2.9.7)，按三目标下载 zip，仅提取 deno/deno.exe；固定 Release 资产 digest 和大小。安装时保留同 tag 的 LICENSE.md（MIT，原作者声明），同样固定 SHA-256；SOURCE.txt 记录下载 URL。
- 下载源只有上述固定 GitHub Release/官方源码地址，无执行远程安装脚本、镜像降级或 PATH 改写。新增工具版本升级须重新核验产物哈希、许可、平台实际执行和本项目媒体接入，不能只更改版本文案。
