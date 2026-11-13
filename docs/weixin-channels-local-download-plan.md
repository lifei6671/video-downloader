# 微信视频号本地解析与下载技术方案

- 日期：2026-11-13
- 项目：lifei6671/video-downloader
- 基线：本地 `master` 分支（制定方案时工作区干净）
- 状态：P0 已通过手动 Cookie 路线的实施门槛；实现与验收以[阶段实测记录](weixin-channels-probe-results.md)为准。
- 定位：桌面应用内完成视频号分享链接的解析、认证和下载；直接调用腾讯相关接口，不依赖公共解析网关

## 1. 产品目标与边界

### 1.1 目标

用户在「下载作品」页面粘贴微信视频号的 `https://weixin.qq.com/sph/...` 分享链接或包含该链接的分享文本，即可预览作品并选择视频格式下载。正常情况下免配置；需要认证时，先使用应用中手动配置的腾讯元宝网页 Cookie，否则尝试读取系统默认浏览器的腾讯元宝 Cookie。读取不到有效登录态时，引导用户在浏览器登录 `https://yuanbao.tencent.com/`，然后重试或手动填写 Cookie。

解析、格式选择和媒体下载均在桌面端进行，复用现有 Tauri + Rust + Python 引擎、yt-dlp 单文件下载、SQLite 缓存、任务队列、取消、重试和文件管理机制。

**重要区别**：没有第三方网关，不等于没有外部依赖。方案仍使用腾讯元宝与微信视频号网页接口。这些属于上游网页接口，不是公开承诺稳定的下载 API；认证状态和接口兼容性必须先用真实样本验证。

### 1.2 第一版范围

- 支持单条 `weixin.qq.com/sph/` 视频分享链接，支持文本中提取单一链接。
- 支持标题、作者、封面、发布时间、时长等可取得的元数据，缺失字段允许为空。
- 支持返回且可验证的 H.264 / H.265 候选；不把编码类型误当作画质等级。
- 支持匿名优先、手动 Cookie、默认浏览器 Cookie；按需触发授权提示。
- 下载前重新解析；复用现有格式选择、进度上报、取消、重试、目录、命名、历史去重。
- 仅支持合法获取且能够按实际验证流程保存为可播放文件的作品。
- 第一版不实现作者主页批量下载、直播、私密内容访问、微信客户端 HTTPS 抓包、根证书注入、系统代理接管、TUN 模式。
- 图集可识别为不支持的内容类型，后续单独扩展。
- 不改变既有抖音、TikTok、YouTube、小红书、Telegram 的下载策略。

### 1.3 实施前明确的事实、假设与未知项

**已从源码确认**

1. `ltaoo/wx_channels_download` 的分享链接路线调用两个接口：
   - `POST https://yuanbao.tencent.com/api/weixin/get_parse_result`：提交 `{type:"video_channel_url",url,scene:1}`，读取 `wx_export_id` 和 `playable_url`；
   - `POST https://channels.weixin.qq.com/finder-preview/api/feed/get_feed_info`：从 `playable_url` 提取 `eid` 和 `token`，用 `{baseReq:{generalToken:token},exportId:eid}` 取得 `feedInfo`。
2. 分享页面的 `feedInfo` 可含 `videoUrl`、`h264VideoInfo.videoUrl`、`h265VideoInfo.videoUrl`；字段是否存在取决于真实响应。
3. 上游项目还提供另一套「微信桌面客户端抓包」路线。该路线通过代理截获客户端信息，可能得到 `decodeKey`，并使用 ISAAC64 异或字节流处理部分文件数据。这不等于分享页接口一定返回 `decodeKey`。
4. 本项目已有 `python-engine/browser_cookies.py`、`src-tauri/src/settings.rs` 和 `src/main.tsx` 的平台 Cookie 配置方案；Python 与 Rust 之间使用单任务单进程的桥接协议 v3。
5. `python-engine/network.py` 的 `GuardedRH` 使用平台域名白名单；`engine.py` 中解析器和平台枚举需要扩展；`parse_cache.rs` 的 `auto` 缓存查询对视频默认假定 `yt-dlp`。

**必须真实验证，不作为既定功能承诺**

- 匿名请求腾讯元宝接口是否成功、返回哪些字段，以及不同时间和网络环境下是否一致。
- 用户登录元宝后，默认浏览器 Cookie 能否被程序成功读取，并满足网页接口的认证要求；是否额外依赖动态请求头或其他会话字段。
- 分享接口返回的视频地址能否直接下载、能否播放、是否需要解密以及能否获取合法有效的密钥。
- `wx_export_id` 或 `eid` 在不同分享链接中是否可作为同一作品的稳定标识。
- 目标 URL 的 CDN 域名、重定向规则、编码与规格信息。
- Windows 11、macOS 下目标浏览器的 Cookie 可读性。
- 本地代理/VPN 场景下上游接口行为与应用现有网络约束是否兼容。

## 2. 设计总览

```text
分享文本 /sph/ 链接
      |
      v
Python: parse_input / identify_input
      |
      v
weixin_channels.py
  ├─ 匿名查询元宝接口
  ├─ 确认需要认证 → 手动 Cookie 或默认浏览器 Cookie → 最多一次认证重试
  ├─ 从 playable_url 提取 eid/token
  ├─ 查询视频号 get_feed_info
  └─ 标准化元数据、稳定身份与媒体候选
      |
      v
现有 Python engine.py / video_formats.py
      |
      v
Rust AppCore / SQLite / 前端预览、格式选择
      |
      v
下载任务获得执行槽位 → 重新解析 → 校验身份与格式
      |
      v
GuardedRH + yt-dlp 单文件下载到受控 staging
      |
      +---- 普通可播放文件 ----> 校验并提交
      |
      +---- 已证实需要解密且可取得密钥 ----> 受控本地解密与校验（后续实施）
      |
      +---- 不具备可验证解密条件 ----> 媒体不支持错误
```

实施架构保持「解析器负责网络内容解析，现有引擎负责媒体传输，Rust 负责任务状态与持久化」。不新增服务端，不引入 Go 运行时，不重建队列。

## 3. 解析与认证设计

### 3.1 输入识别

- 仅接受 HTTPS 且主机严格等于 `weixin.qq.com`，路径为经样本验证的 `/sph/<token>` 格式。
- 沿用现有 `parse_input` 的单链接提取和输入校验，去除分享文案及无关追踪参数，禁止用户名/密码、异常端口、非 HTTP(S) 方案。
- 分享令牌不直接视为作品永久 ID；保留原始可重试的规范化分享 URL。
- `identify_input` 对视频号不能沿用「直接从 URL 取末段并返回永久身份」的快速路径。需要网络解析以取得受验证的身份，或者采用明确的分享链接级临时身份策略。

### 3.2 本地两段请求

- 第一段：向 `yuanbao.tencent.com/api/weixin/get_parse_result` 发送规范化分享 URL，解析 `wx_export_id`、`playable_url`。响应缺少关键字段时返回 `WEIXIN_UPSTREAM_CHANGED`，不得拼凑未知令牌。
- 第二段：从 `playable_url` 严格解析 `eid`、`token`；向 `channels.weixin.qq.com/finder-preview/api/feed/get_feed_info` 发送 `baseReq.generalToken` 和 `exportId`；检查 HTTP 状态与业务 `errCode`、`data.errMsg`，提取作者、视频、封面和可用格式。
- `playable_url` 只作为参数容器解析，不直接自动打开或请求其未知目标；对主机、路径和查询字段设置合理长度上限。
- 独立使用编码正确的 JSON 序列化和 URL 编码，不用拼接字符串构造请求体。
- 上游实现中某些固定 `x-id`、`x-agentid`、`x-device-id`、`x-webversion` 等字段来自网页请求样本；不把这些固定个人或会话值当作长期稳定的认证协议。仅在真实验证证明必要且可合法生成时实现相应字段。
- 禁止将 `wx_export_id`、`token`、完整 CDN URL、Cookie 或上游原始响应写入可公开的日志或前端事件。

### 3.3 认证状态机

```text
ANONYMOUS
  ├─ 成功 → 查询详情 → RESOLVED
  ├─ 明确认证不足 → AUTH_RETRY
  ├─ 视频不存在/访问受限 → 内容错误
  └─ 超时/风控/字段变化 → 对应错误，不误判为需要登录

AUTH_RETRY
  ├─ 用户已手动配置元宝 Cookie → 用手动 Cookie 重试一次
  ├─ 没有手动配置 → 从系统默认浏览器提取元宝适用 Cookie，重试一次
  └─ 无有效 Cookie → LOGIN_REQUIRED

RETRY_RESULT
  ├─ 成功 → RESOLVED
  ├─ 认证失败 → LOGIN_REQUIRED 或 COOKIE_EXPIRED
  └─ 其他错误 → 保留实际错误类型
```

规则：

1. **匿名优先**：即使用户已配置 Cookie，也先尝试匿名，只有明确需要认证时才使用凭证。这样可以减少敏感 Cookie 的网络发送。
2. 手动 Cookie 优先于浏览器 Cookie；遵循现有抖音配置语义，手动配置失效时提示用户更新，不在背后悄悄切换账号。
3. 仅认证相关错误触发 Cookie 读取，禁止在普通 HTTP 403、404、限流、内容不可访问时一律认为是 Cookie 缺失。
4. 默认只读取当前操作系统默认浏览器；不扫描所有浏览器，不自动读取其他平台的登录态。
5. 打开腾讯元宝登录页由用户完成；软件提供「打开元宝登录页面」「重试解析」「手动配置 Cookie」入口，不要求创建内嵌浏览器登录器。
6. 不能读取浏览器 Cookie 时允许手动配置；自动读取的 Cookie 只用于本次请求，不写入 SQLite。
7. 已有手动 Cookie 的存储和读取保持项目现有行为；不得通过错误事件、前端任务快照或诊断日志暴露明文。
8. 不使用视频号微信登录 Cookie 冒充元宝 Cookie；本解析路线的认证域为 `yuanbao.tencent.com`。

### 3.4 浏览器 Cookie 精确隔离

改造 `python-engine/browser_cookies.py`：

- 保留现有 `default_browser()`、`extract_cookies_from_browser()` 和现有平台逻辑。
- 添加按 **认证目标域名** 提取 Cookie 的通用接口，例如 `load_default_cookies_for_host(jar, "yuanbao.tencent.com", logger)`。基于 Cookie 的 Domain / Path / Secure / Expires 属性计算适用范围；正确处理作用于 `yuanbao.tencent.com` 的父域 Cookie，不扩大到整个腾讯域下的其他站点。
- 不再用 `platform + ".com"` 来定位元宝 Cookie，不把 `weixin` 平台误映射为 `weixin.com`。
- 使用隔离的元宝请求 Cookie 容器；发送到视频号详情接口与媒体 CDN 的请求不得继承元宝认证 Cookie。
- 浏览器提取失败要区分浏览器不可识别、Cookie 不存在、读取失败、认证过期；面向 UI 只输出固定错误码，不输出浏览器数据库或系统密钥库路径。
- 校验在 Windows/Edge 或 Chrome、macOS/Safari 或 Chrome 下实际有效；如果浏览器机制不支持则提供手动配置路径。

手动 Cookie 从已有 `settings.rs` profile 快照传入 Python 引擎，仅在认证重试时作为请求头使用。保留手动 Cookie 长度、换行符和非法格式检查。

### 3.5 网络与代理

「系统代理抓包」和「应用请求网络代理」是两类不同能力。

- 本项目只使用已有平台级 HTTP/SOCKS5 下载代理配置，用于应用主动发起的元宝、视频号及 CDN 请求。
- 不接入上游 `wx_channels_download` 的透明抓包代理、根证书、系统路由或 TUN。
- 在 `network.py` 定义解析请求与媒体下载请求的不同目标白名单，并逐跳校验重定向。解析目标先限制为 `yuanbao.tencent.com`、`channels.weixin.qq.com`；媒体 CDN 域名由真实样本确认后最小化添加。
- 不信任解析返回的任意 URL，拒绝 loopback、链接本地地址、私网目标、携带用户信息的地址及非预期端口；保留项目既有代理/TUN 网络兼容策略，避免误改其他平台的网络路径。
- 限制响应大小、超时、最大跳转次数及错误内容长度；禁止自动访问未知主机。
- 代理配置默认保持现有 UI 的「直连 / 自定义代理 / 绕过代理」语义，不新增系统代理设置。

## 4. 解析结果、身份与格式选择

### 4.1 标准元数据

映射到现有 `Video` 任务模型：

```json
{
  "platform": "weixin",
  "video_id": "经验证稳定的作品ID，或明确标记的分享链接级ID",
  "canonical_url": "https://weixin.qq.com/sph/...",
  "source_url": "https://weixin.qq.com/sph/...",
  "resolver": "weixin",
  "kind": "video",
  "title": "示例视频标题",
  "author": "示例作者",
  "cover_url": null,
  "duration_ms": null,
  "formats": [],
  "selected_format_id": null
}
```

- `video_id` 不使用会过期的 CDN URL、标题或视频编码名称。
- 优先验证 `wx_export_id` 或详情接口的作品 ID 是否稳定、同作品多分享链接是否一致；若无法证明跨链接稳定，可临时采用 `sha256("weixin:sph:" + normalized_share_token)` 作为**分享链接级标识**，明确只保证同链接去重，不能宣称跨分享链接去重。
- 重试时使用 `canonical_url` 重新解析，核对已存的身份策略与可得的真实标识；不能以随机临时令牌作为持久身份。
- 缺少作者、时长等字段不阻断正常下载；缺少真实媒体 URL 则必须阻断。

### 4.2 格式规范

从 `h264VideoInfo.videoUrl`、`h265VideoInfo.videoUrl` 和 `videoUrl` 构造候选；去除重复 URL，不凭空宣称编码或画质。

- 只有明确字段或媒体探测能确认 H.264/H.265 时才显示对应编码；未知时显示「编码未知」。
- 优先 H.264 是**兼容性默认偏好**，不是画质等级；如果真实分辨率、码率、帧率不同，使用当前 `video_formats.py` 的排序字段评估，但要显式处理兼容性偏好。
- 复用 `format_key` / `public_formats` / `select_format`，确保用户选中规格在下载前重新解析后仍能按**稳定媒体属性**匹配；不能将签名 URL、到期时间或随机 CDN 参数纳入规格身份。
- 缺失分辨率/码率时保持 `null`，不能编造 `1080p`、大小和音轨。
- 校验媒体是否包含音轨；现有单文件格式选择器可能过滤无音轨视频，必要时按实际音轨信息适配，不误当作下载失败。
- 不在预览、Rust 数据库、任务事件和日志中输出签名媒体 URL。媒体 URL 只存在本次 Python 工作进程内。

### 4.3 缓存与下载重解析

- 复用 `parse_cache` 的缓存元数据、封面与别名映射。
- 扩展 `auto` 解析器匹配规则：`weixin/video` 必须匹配 `weixin`，不能继续走视频默认 `yt-dlp` 假设。
- 缓存公开标题、作者、封面、规范化分享链接、格式摘要；不缓存 Cookie、`generalToken`、`eid` 或签名媒体 URL。
- 下载真正开始时重新请求上游并严格验证视频身份、所选规格，媒体 URL 过期时按身份重新解析并有限重试。
- 分享链接失效时返回可重试或内容失效错误，不使用缓存中可能已过期的媒体地址。
- 为 `weixin` 增加解析器版本修订，避免未来上游响应变更复用不兼容缓存。

## 5. 媒体下载与解密

### 5.1 普通视频

- 尽量复用 `yt-dlp` 的受控 HTTP 单文件下载、进度回调、暂停/取消路径和 staging 输出约定；先以可解析视频类型构造单文件内部信息，确认现有 `GuardedRH` 可校验 CDN。
- 现有实现设置 `continuedl=False`，不要承诺已经支持视频号断点续传；如未来要启用须单独设计 Range 和重新签名兼容性。
- 文件实际落盘后验证非零大小、预期媒体容器和可播放性；数据错误或 HTML 错误页不得标记为成功。
- 保持 Rust 现有 staging、文件名和原子提交契约。

### 5.2 受保护视频

源码核查结论：

- 上游 `pkg/scraper/wxchannels/decrypt.go`、`reader.go` 使用 ISAAC64 生成密钥流并按字节异或；当前调用常见加密区域大小为 131072 字节。
- 上游抓包模型中 `objectDesc.media[*].decodeKey` 可用于该解密流程；分享页面 `SharedFeedinfo` 的当前字段定义只有视频地址等字段，**没有证据表明所有分享解析都提供 `decodeKey`**。
- H.264/H.265 视频编码不等于加密方案；不能根据 H.265 或容器头判断一定需要解密。

实施约束：

1. 首版先验证分享接口下载的媒体是否为可播放文件。
2. 若媒体加密且接口没有提供经过验证的解密材料，返回 `WEIXIN_MEDIA_PROTECTED`，保留错误记录；不伪装成功。
3. 只有在接口能合法获得密钥、加密类型和作用范围，且测试证明恢复正确时，才实现本地解密处理器。
4. 若实现，应只在任务 staging 内工作，原始下载完成后进行解密与媒体完整性验证，成功后交给 Rust 提交文件；失败时不会覆盖用户正式文件。
5. 解密实现必须有至少 2 个公开可复现/已授权的加密样本和对应原始视频对照，测试首部、边界、文件长度、错误密钥、重复执行。
6. 不把解密逻辑当作通用 DRM（数字版权管理）绕过能力，不承诺覆盖所有平台保护方式。

### 5.3 上游许可与代码引用

- 本项目根目录许可证为 Apache-2.0；参考项目 `ltaoo/wx_channels_download` 的根许可证是 **MIT + Commons Clause**，带有商业使用/销售限制，并非无附加条件的纯 MIT。
- 先核对所引用源码的实际许可及第三方来源；**不要未经确认直接复制**它的 Worker、Go 解密器或其他大段实现到当前发行项目中。
- 优先以接口行为、公开协议字段和独立测试为依据在本项目重新实现；涉及受限源代码改编、复制或分发时，完成许可证兼容性审查，保留必须的授权和署名信息。
- 参考原始实现：
  - https://github.com/ltaoo/wx_channels_download/blob/main/pkg/scraper/wxchannels/yuanbao.go
  - https://github.com/ltaoo/wx_channels_download/blob/main/internal/workers/sph/worker.js
  - https://github.com/ltaoo/wx_channels_download/blob/main/pkg/scraper/wxchannels/decrypt.go
  - https://github.com/ltaoo/wx_channels_download/blob/main/pkg/scraper/wxchannels/reader.go
  - https://github.com/ltaoo/wx_channels_download/blob/main/LICENSE
  - https://github.com/tjxj/z-skills/blob/main/z-video-downloader/scripts/download_video.py

## 6. 项目代码改造清单

| 位置 | 修改内容 |
| --- | --- |
| `python-engine/weixin_channels.py`（新建） | 两段 HTTP 解析、鉴权状态机、业务错误校验、元数据与媒体候选映射 |
| `python-engine/browser_cookies.py` | 按元宝实际认证域读取默认浏览器 Cookie，保留既有平台实现 |
| `python-engine/engine.py` | 输入识别、`identify_input` 网络身份流程、`weixin` 解析器、下载前重解析、格式与错误处理 |
| `python-engine/network.py` | 元宝接口、视频号接口与媒体 CDN 分级白名单、重定向与目标校验 |
| `python-engine/video_formats.py` | 复用现有格式键与格式展示；补充视频号候选适配测试 |
| `src-tauri/src/settings.rs` | 新增 `weixin` profile，沿用现有 SQLite Cookie 和可选网络代理配置 |
| `src-tauri/src/desktop_core.rs` | `weixin` 解析器枚举、下载身份验证、历史筛选、重试、解析配置快照 |
| `src-tauri/src/parse_cache.rs` | `auto` 匹配、分享链接别名、缓存修订与安全字段过滤 |
| `src-tauri/src/python_bridge.rs` | 引擎版本同步、错误码白名单以及下载结果协议一致性 |
| `src/api.ts` | `Resolver`/平台类型及相应显示字段 |
| `src/main.tsx` | 平台名称、解析器选项、账号 Cookie 设置、登录提示、预览规格文案 |
| `src/components/task-filters.tsx` | 新增视频号任务筛选 |
| 测试目录 | Python 单元/HTTP 模拟测试、Rust 缓存/任务回归、前端交互 |
| `docs/weixin-channels-local-download-plan.md` | 记录验证结果、决策、实施与验收结论 |

同时全局搜索：`douyin | tiktok | youtube | xiaohongshu | telegram` 的枚举、`Resolver`、`Profile`、错误码翻译、下载历史清理、文件命名模板、备份元数据和应用引擎握手，逐一检查新增平台影响。

除非真实验证要求，不扩大 `bridge.py` 的 method 集合，继续通过现有 `identify_input`、`resolve_video`、`download_video` 完成工作。协议 v3 可保持；引擎版本需同时更新 Python `ENGINE_VERSION` 与 Rust `python_bridge.rs` 约束，避免握手不匹配。

## 7. 错误码和用户体验

推荐新增并纳入 Rust/前端统一错误翻译：

| 错误码 | 条件 | UI 文案 / 行动 |
| --- | --- | --- |
| `WEIXIN_LOGIN_REQUIRED` | 需要认证，且没有可用 Cookie | 请在默认浏览器登录腾讯元宝后重试，或手动配置 Cookie |
| `WEIXIN_COOKIE_EXPIRED` | 已提供 Cookie，但认证失败 | 腾讯元宝登录信息已失效，请更新后重试 |
| `BROWSER_COOKIES_UNAVAILABLE` | 默认浏览器 Cookie 无法读取 | 可在平台账号设置中手动填写 |
| `WEIXIN_VIDEO_UNAVAILABLE` | 作品失效/不可访问 | 当前视频不可访问 |
| `WEIXIN_RATE_LIMITED` | 上游明确限流 | 请求频繁，请稍后重试 |
| `WEIXIN_UPSTREAM_CHANGED` | 上游响应字段变化 | 视频号解析接口发生变化 |
| `WEIXIN_MEDIA_EXPIRED` | 媒体地址过期 | 重新获取媒体地址后重试 |
| `WEIXIN_MEDIA_PROTECTED` | 文件受保护但缺少可验证的处理条件 | 当前视频的媒体格式暂不支持 |
| `NETWORK_TARGET_REJECTED` | 请求或重定向目标不符合白名单 | 网络目标校验未通过 |

- 只有确认证书失效、超时、暂时性限流等可恢复失败才重试；认证失败最多一次切换 Cookie 重试，避免反复触发上游风控。
- 第一版 UI 复用平台账号现有表单，显示「微信视频号（通过腾讯元宝认证）」；提供打开元宝登录页入口。
- 手动 Cookie 标签明确为「腾讯元宝网页 Cookie」，并说明不是微信 Cookie。
- 各字段显示客观事实，未知分辨率、时长、文件大小保持未知。
- 在前端登录提示旁提供一键重试，不把浏览器 Cookie 的实际内容展示在下载结果中。

## 8. Codex 分阶段执行步骤

### P0：可行性实验（必须先完成，暂不接 UI）

实现一个单独的、可调用的 Python 诊断入口（例如 `scripts/probe_weixin_channels.py`），复用既有网络约束和浏览器 Cookie 工具，接受由开发者提供的测试分享 URL。不要向版本库提交真实 Cookie、个人会话头、原始签名 URL。

实验矩阵：

1. 无 Cookie 请求元宝解析接口，区分匿名成功、认证不足、内容无效、限流、接口变更。
2. 在浏览器登录元宝后读取默认浏览器 Cookie 进行请求。
3. 从手动配置的 Cookie 发起同一请求。
4. 对成功的元宝响应提取 `playable_url`、`eid`、`token`，调用第二段详情接口，验证其错误码和字段结构。
5. 分别下载一段 H.264/H.265 示例媒体到临时目录，使用媒体探测检查容器、视频轨、音轨和可播放性。
6. 检查加密标志与所需密钥是否存在，记录直接下载与受保护资源的数量。
7. 用同一作品的两个分享链接验证作品身份稳定性。
8. Windows 11 和 macOS 上验证默认浏览器 Cookie 读取结果；未验证的标记为待验证。

**P0 交付**：`docs/weixin-channels-probe-results.md`，包含脱敏请求结构、各实验状态、字段说明、成功与失败样本统计、加密结论、身份方案决策。没有完整可下载样本时，不把功能宣布为可用，也不要进入正式任务状态集成。

### P1：本地解析模块与测试

- 增加 `weixin_channels.py`、请求超时/响应上限/错误处理、认证重试、域名验证。
- 增加可脱敏的模拟接口响应 fixture，覆盖匿名成功、认证失败、结构变化、内容失效、Cookie 缺失、CDN URL 不合法等情形。
- 按 P0 确定身份与规格策略；不把暂时 URL 暴露给 Rust。

### P2：Rust/Python 引擎集成

- 接通 `identify_input` / `resolve_video` / `download_video`。
- 复用现有 `yt-dlp` 下载进度与 staging 验证路径；不破坏其他平台。
- 补充 Rust 的缓存匹配、任务身份、格式选择、重试、历史筛选及引擎版本映射。
- 添加分享链接缓存别名和恢复策略测试。

### P3：前端与设置集成

- 增加视频号显示名称、账号卡片、自动读取默认浏览器 Cookie、手动 Cookie、登录/重试提示。
- 复用当前解析器选择和格式选择 UI；当输入为视频号时自动选用专用解析器。
- 视频预览、取消、失败重试、任务历史和筛选完整贯通。

### P4：系统验证与发布判定

- Python 单元测试、Rust 相关单元/集成测试、前端构建与类型检查。
- Windows 11 实机 + macOS 实机，验证匿名、有 Cookie、Cookie 失效、默认浏览器不可读、代理开关、文件取消和重试。
- 运行抖音视频、抖音图集、TikTok、YouTube、小红书、Telegram 的既有回归测试。
- 如 P0 发现媒体加密且有可验证的解密材料，可新增单独子阶段实现受控解密；否则按受保护媒体错误处理。

## 9. 验收标准

### 9.1 功能

- 全新安装用户粘贴可匿名访问的视频号链接时，能够解析、预览和下载可播放文件。
- 需要认证时，提示登录元宝；用户登录默认浏览器后可重试成功，或通过手动 Cookie 成功。
- Cookie 提取失败、手动 Cookie 失效及内容本身不可访问能给出不同提示。
- 同链接重复解析能够命中安全缓存；下载开始时重新获得有效媒体地址。
- H.264/H.265 规格仅在确实存在且可验证时显示；选择的格式不会被重新解析错误切换。
- 文件下载完成后可用媒体探测打开；下载中取消不产生已完成记录。
- 退出/重启后的任务恢复、失败重试、历史筛选和删除行为符合现有实现。

### 9.2 安全、兼容与质量

- 无 Cookie、播放令牌、解密密钥、签名 URL 进入普通日志、前端事件或 SQLite 公共任务快照。
- 元宝 Cookie 只发送到符合 Cookie 作用域的元宝请求，不发送到视频号详情接口或媒体 CDN。
- 拒绝私网/本地重定向和未验证媒体目标；代理行为不影响其他平台。
- 没有新增系统级根证书、系统代理接管、TUN、第三方解析网关及 Go 运行时。
- Windows 11、macOS 实机通过；浏览器 Cookie 不可读时手动配置仍可使用。
- 受保护媒体不能被误判为下载成功；不以修改文件扩展名掩盖无效数据。
- 所有既有平台回归通过，且当前工作区既有用户改动得到保留。

### 9.3 样本与结果记录

准备至少 20 条**有权下载且允许测试**的视频号链接，包括：同一作品不同分享链接、正常视频、多编码格式、失效作品、认证要求、错误/超时、媒体过期、网络代理场景。样本清单在本地保存，提交前脱敏，尽量使用模拟 fixture 支撑可重复测试。

记录：日期、操作系统、认证方式、接口响应类型、可选格式、文件可播放状态、是否加密、身份稳定性、实际错误码。不得凭空宣称匿名访问率、支持率或成功率。

## 10. Codex 执行约束与交付

1. 读取当前本地工作区现状，先检查 `git status` 和相关模块，不覆盖已有用户修改。
2. 以本方案为实现基准，但以真实接口验证数据校正未经证实的技术假设。
3. P0 必须先完成；如匿名访问、登录态认证或媒体下载关键链路存在阻塞，记录证据与具体下一步，不用第三方公共解析接口掩盖问题。
4. 不直接整体移植上游带许可限制的 Go/Worker 项目；关注许可证和归属，保持项目现有 Apache-2.0 许可证要求。
5. 仅在源码、依赖、测试和文档形成可重复的成果后，进入下一阶段；逐阶段说明实际完成项与验证状态。
6. 交付正式代码、测试、`docs/weixin-channels-probe-results.md`、必要的用户文档和可追溯的测试结果。

## 11. 参考资料

- 本项目：https://github.com/lifei6671/video-downloader
- 原始 Skill：https://github.com/tjxj/z-skills/blob/main/z-video-downloader/SKILL.md
- 原始 Skill 下载代码：https://github.com/tjxj/z-skills/blob/main/z-video-downloader/scripts/download_video.py
- 视频号解析上游：https://github.com/ltaoo/wx_channels_download
- 分享接口 Go 实现：https://github.com/ltaoo/wx_channels_download/blob/main/pkg/scraper/wxchannels/yuanbao.go
- 解析 Worker：https://github.com/ltaoo/wx_channels_download/blob/main/internal/workers/sph/worker.js
- 公开的分享详情结构：https://github.com/ltaoo/wx_channels_download/blob/main/pkg/scraper/wxchannels/types.go
- 本地解密实现：https://github.com/ltaoo/wx_channels_download/blob/main/pkg/scraper/wxchannels/decrypt.go
- 分段读取处理：https://github.com/ltaoo/wx_channels_download/blob/main/pkg/scraper/wxchannels/reader.go
- 上游代理工作模式：https://github.com/ltaoo/wx_channels_download/blob/main/docs/cli/proxy.md
- 上游许可证：https://github.com/ltaoo/wx_channels_download/blob/main/LICENSE
