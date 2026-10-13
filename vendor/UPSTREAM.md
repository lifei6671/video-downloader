# yt-dlp 来源与依赖

当前平台解析和媒体下载统一使用官方 Python 包 `yt-dlp==2026.8.19`；运行时版本 `2026.08.19`，协议 v2，引擎版本 `0.2.1`。

- 来源：https://github.com/yt-dlp/yt-dlp/tree/2026.08.19
- PyPI：https://pypi.org/project/yt-dlp/2026.8.19/
- 官方 wheel：`yt_dlp-2026.8.19-py3-none-any.whl`
- wheel SHA-256：`1d57897e94c6665a0a6f9bc54b34e584284e32c034ffab3a7df25d8f7b24eedf`（PyPI 发布元数据核对）
- sdist SHA-256：`9e213e48cea35c66b378e4447903f118f6392a5fa380a2b6d7070ec86f4e0af1`
- Unlicense 原文保留于 `yt-dlp-LICENSE`，从已安装的对应 wheel 许可证复制。

不修改已安装的 yt-dlp 包；经用户明确批准，在自有适配层覆盖 DouyinIE 的单一详情请求构造，补丁范围见下文。`python-engine/engine.py` 使用 `YoutubeDL` 嵌入接口；`network.py` 为本应用的双平台网络边界适配，固定依赖该版本的 `UrllibRH`、`HTTPHandler` 和请求 director。沿用上游 Cookie、HTTP 解码、重定向及 SOCKS 实现，只增加逐跳目标校验与直连 DNS 结果固定；升级须重测这些内部接入点。显式 HTTP 代理由适配层设置路由，避免环境 NO_PROXY 或系统 bypass 改走直连；SOCKS 保留上游实现。远端 DNS 代理的实际目的 IP 无法由本客户端验证。

运行依赖仅为 yt-dlp，不启用 extras；没有 wreq、httpx、pydantic、structlog、reqwest、dtk 或 FastAPI/Redis 等服务端依赖。开发锁文件另有 pytest、PyInstaller 及其依赖。Python 3.12 与 Windows x64 为当前已执行环境，macOS 尚未执行。

旧 dtk 源码、manifest、初始化补丁、旧许可证副本与专属签名/parser fixtures 已删除；没有保留旧引擎兼容或下载 fallback。仓库根 LICENSE 属于本项目，保持原样。少量旧忽略缓存的删除受自动审批限制；缓存不在依赖、源码、构建 pathex 或分发包中。

官方 PyInstaller hook 收集打包所需模块；产品运行只注册两平台单视频提取器。发布时同时分发完整 onedir、Python/yt-dlp 许可证与实际第三方声明，不单独复制 exe。当前尚非完整 Tauri 安装包。

测试、真实在线结果及未通过项见 [M1 验证记录](../docs/m1-validation.md)。

## Douyin 详情请求补丁

2026-10-13 用户明确允许最小签名补丁，作为项目“不重写签名算法”约束的单一例外。`python-engine/douyin.py` 继承固定版本 DouyinIE，仅为 `https://www.douyin.com/aweme/v1/web/aweme/detail/` 添加请求参数，不替换其 `_real_extract`、媒体 parser、格式选择或下载器。

- 问题背景：[yt-dlp #9667](https://github.com/yt-dlp/yt-dlp/issues/9667)。本机原始与 guarded transport 对照均获得 `ArgusSecurityPlugin Uifid Not Found`，加入 Cookie 中 UIFID 后变为 `Signature Not Found`，加入正确签名后返回 200/status_code=0。
- 公式来源：[Evil0ctal 原始研究提交说明](https://github.com/Evil0ctal/Douyin_TikTok_Download_API/commit/d9f718800df20c0255979d93ebadb33dd3a3b757)。仅依据公布公式独立实现，没有复制 dtk 源文件、签名模块或恢复其依赖；使用 Python 标准库，不新增第三方包。
- 读取授权 Cookie 的 UIFID（或 UIFID_TEMP）；不生成游客身份、不调用浏览器或远程签名服务。请求包括 aweme_id、uifid；timestamp 不发送，签名中的 timestamp 字段为空。
- 签名为 `MD5(uifid + "_" + timestamp + "_" + salt + "_" + URLSearchParams(query))`；固定 salt 为该研究公布的 `A96D855A08C0A9707F8BEF0D9A527E4E`。编码按浏览器规则处理空格、星号、波浪号，测试向量由 Node URLSearchParams/crypto 独立生成。
- 未持有 UIFID 时请求前报错；Cookie、签名 query 和媒体 URL 不记录日志、不持久化。域名 `365yg.com` 来自本次签名成功详情返回的抖音视频 CDN，只加入抖音清单；每跳目标及直连 DNS 检查仍生效。
- 上游加入可验证修复后优先移除此覆盖。固定版本升级或平台 salt/参数变化时，重新运行补丁测试、双平台实际下载、解码和打包验证；不自动切回旧引擎，不声称补丁永久有效。

## M2 桌面界面

生产界面使用 shadcn/ui 官方 radix-nova 注册表生成的组件，来源 https://ui.shadcn.com/r/styles/radix-nova/ ，CLI 4.21.0；组件保存在 `src/components/ui/`，MIT 许可副本见 `shadcn-LICENSE`（来自安装包 LICENSE.md）。仅 Progress 透传 value 到 Radix Root，以提供真实 ARIA 进度。页面仅组合这些组件与业务数据，不另建基础控件。React、Radix、Tailwind、Lucide 和 Tauri 的实际版本/完整性哈希由 package-lock.json 与 Cargo.lock 固定；没有运行时 CDN。
