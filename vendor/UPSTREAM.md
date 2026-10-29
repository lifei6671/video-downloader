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

2026-10-19 用户批准补充抖音规格类型归一化。实测作品 `7664441562780702287` 的 `video.bit_rate` 同时包含 `format=mp4` 和 `format=dash`；yt-dlp 2026.08.19 的 `_parse_aweme_video_app.extract_addr` 默认填入 `acodec=aac`，没有处理分离音轨。自有 Douyin 子类继续调用上游 parser，再按具体 URL 关联每档 `video_extra`（JSON 字符串或对象）：MP4 与正数声道/采样率证据一致时标一体；DASH 的 `audio_file_id` 匹配 `bit_rate_audio[].audio_meta.file_id` 时纠正为无内嵌音轨；独立音轨及 `music.play_url` 标为音频；证据缺失、冲突均为未知。数字字符串按有限正数解析，布尔值不充当声道数。

类型标记仅留在 Python 内部。现有选择器只向抖音 yt-dlp 的预览/下载提供已确认的一体规格，不新增音频下载、分轨合并、媒体探测请求或公开协议字段。引擎 0.7.2 使旧解析缓存失效，旧规格失效时明确失败，不自动降级。此补充来源为本机授权详情响应与 FFprobe 轨道核对，没有复用 DownVid 固定 `hasAudio=true` 的实现。升级 yt-dlp 时重新检查上述字段及真实轨道；上游正确识别且项目所需的一体证据完整时移除此归一化，保留回归测试。签名及 GuardedRH 均未改变。验收见 [视频规格](../docs/video-format-validation.md#抖音音视频类型修正2026-10-19)。

## 抖音封面优先级补充（2026-10-21）

按用户要求，抖音视频优先使用详情响应的 `video.cover_original_scale.url_list` 中的 HTTPS 地址，字段缺失或没有可用地址时沿用各引擎原有封面逻辑。yt-dlp 的自有 Douyin 子类补充该候选，选择器将其置于尺寸、`origin_cover` 和上游 preference 之前；DTK 自有适配层从已解析内容的 raw 详情读取同一字段，不修改 vendored parser。TikTok、YouTube 和图集保持原有行为。

来源为作品 `7697086330987981242` 的本机授权详情响应：`origin_cover` 是橙字热榜画面，`cover_original_scale` 是白字设计封面，不能按字段名称将两者视为同一图片。引擎升为 0.7.3，使后续解析不再命中旧版本缓存；不主动替换历史任务封面。预览与下载复用同一选择结果及原有图片校验，封面下载失败不影响视频成功。升级上游时核对该字段是否已原生提取，保留优先级与缺失回退测试。

### 同画面高清静态封面（2026-10-24）

用户批准在原有封面基准上选择同画面的高清静态候选。实测作品 `7699844701738235301` 的 `cover_original_scale` 为 360×480 JPEG，`dynamic_cover` 为同画面的 766×1022 静态 JPEG，而 `origin_cover` 是不同画面的 360×640 JPEG；响应声明尺寸与实际图片也不一致，不能按字段名或声明尺寸选图。

自有 `douyin.cover_candidates` 从 `cover_original_scale`、`dynamic_cover`、`origin_cover`、`cover` 各取第一个 HTTPS 地址并去重，不拼接或改写 CDN 模板，不修改 vendored parser。yt-dlp 和 DTK 视频路径均将这些候选仅保留在 Python 内存；先按既有 `cover_url` 取得基准，基准不可用时保留原有失败结果。候选请求沿用 GuardedRH、代理和凭据，最多增加 4 次（通常 3 次），每次使用既有 socket timeout 扩展设为 5 秒，单图仍限 10 MiB；候选失败不影响基准封面和视频。

复用 [Pillow 13.0.0](https://github.com/python-pillow/Pillow) 的解码、EXIF 方向校正、Lanczos 缩放和 ImageChops/ImageStat 比较，不依赖用户安装 FFmpeg。只比较不超过 2000 万像素、不带透明通道的单帧 JPEG/PNG/WebP；候选宽高均不小于基准且像素数更大，宽高比偏差不超过 1%。在 128×128 RGB 归一化图上，各通道整体平均绝对差不超过 4/255，所有 16×16 分块的通道均差不超过 10/255，才视为同图候选；始终与原基准比较，选择符合条件的最大图。该规则为保守近似匹配，不能证明像素内容完全相同；无法确认时保留基准。动画、不同画面、显著文字改动、模糊专用封面和失败候选不替换。保存获选图片的原始字节，不转码、不把比较缩略图写入缓存。

Pillow 精确锁定并随 sidecar 分发，PyInstaller 一并收集其许可。引擎 0.13.1、协议仍为 v3，使后续解析不命中旧引擎缓存；已有历史任务封面不自动批量替换。图集、TikTok、YouTube、Telegram、存储及删除语义保持。对应测试为 `tests/test_douyin_cover.py`，升级上游或 Pillow 时核对格式、实际尺寸、同图与不同图及原有回退。

## Telegram tdl 外部工具（2026-10-18）

按用户决定只调用用户自行安装的 [iyear/tdl v0.20.4](https://github.com/iyear/tdl/tree/9d7d49eef2bcb04da720c26e33598c49c68b9ddd)，commit `9d7d49eef2bcb04da720c26e33598c49c68b9ddd`，上游许可 AGPL-3.0。未 vendoring、复制其 Go 源码或随安装包分发 tdl；自有 Rust 适配器依据 `app/dl/serve.go`、`serve.go.tmpl`、`cmd/dl.go`、`cmd/version.go`、`cmd/root.go` 和 `core/util/tutil/tutil.go` 的 CLI/HTTP 契约实现。用户另行授权运行依赖登录，依据 `cmd/login.go`、`app/login/login.go`、`app/login/qr.go` 调用显式 QR 模式和独立存储；禁止默认 desktop 导入及覆盖既有命名空间。当前全接口监听、缺少令牌及请求白名单须作为已知实验风险持续呈现；升级和真实运行边界见 [集成说明](../docs/tdl-integration.md)。

### tdl 界面内认证桥接（2026-10-18）

仍执行用户安装的 tdl v0.20.4 `login --type qr`，不修改其 Go 代码。上游没有 JSON 认证接口；隐藏伪终端复用 `app/login/qr.go` 的二维码/2FA 流程，二维码像素还原依据 `skip2/go-qrcode` commit `da1b6568686e` 的 `ToSmallString(false)`，未复制其实现源码或引入 Telegram 认证库。

新增 Rust 直接依赖：`portable-pty 0.9.0`（MIT）负责 Windows ConPTY / Unix PTY，`vt100 0.16.2`（MIT）处理终端重绘及屏幕状态，`zeroize 1.9.1`（Apache-2.0 OR MIT）清理 Rust 临时密码内存。完整版本/校验值见 Cargo.lock；新增及直接使用依赖的许可文本见 `vendor/tdl-ui-licenses.txt`，随安装包放入 `licenses/tdl-ui-licenses.txt`。不新增 shell 登录脚本，不转发终端原文、二维码 URL 或密码到日志/持久化。

## 运行依赖下载（2026-10-18）

用户明确授权运行依赖页下载缺失工具，替代此前“只检测、不下载”的约束。仅作为应用专属外部工具，不更新内置解析引擎。

- FFmpeg/FFprobe：采用 [eugeneware/ffmpeg-static b6.1.1](https://github.com/eugeneware/ffmpeg-static/releases/tag/b6.1.1) 的 Windows x64 / macOS x64、arm64 gzip 静态构建。Release 标签与二进制版本分别记录：Windows x64 / macOS x64 为 6.1.1，arm64 为 6.0.0，安装器按平台验证。SHA-256、压缩大小直接核对该 Release 的公开资产元数据，与 Downvid 固定 manifest 对照一致；没有复用其源码。下载保留各平台 LICENSE、README（包含构建/源码说明），二者亦校验 SHA-256。许可依实际构建的 LGPL/GPL 声明，不将静态构建统一描述为 MIT。资产来源及校验值见 `python-engine/runtime_assets.py`。
- Deno：采用官方 [denoland/deno v2.9.7](https://github.com/denoland/deno/releases/tag/v2.9.7)，按三目标下载 zip，仅提取 deno/deno.exe；固定 Release 资产 digest 和大小。安装时保留同 tag 的 LICENSE.md（MIT，原作者声明），同样固定 SHA-256；SOURCE.txt 记录下载 URL。
- 下载源只有上述固定 GitHub Release/官方源码地址，无执行远程安装脚本、镜像降级或 PATH 改写。新增工具版本升级须重新核验产物哈希、许可、平台实际执行和本项目媒体接入，不能只更改版本文案。

## Fun-ASR-Nano 本地转录（2026-10-23，当前）

按用户更换模型要求，当前改用下节所列同一固定 FunASR / llama.cpp 源码基线的 `llama-funasr-cli`，由 `scripts/build_asr.py` 构建。`vendor/asr-windows.manifest` 保留 UTF-8 路径支持；CPU helper 与独立 `llama-funasr-vad` 使用相同固定基线构建。Windows 可选 audio.cpp CUDA，macOS 保留 CPU，不再调用 SenseVoice CUDA 组件。

模型来自 [FunAudioLLM/Fun-ASR-Nano-GGUF](https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-GGUF/tree/46e849502a867080d66d351b8dfb1018b607e509)：F16 编码器、Q8_0 解码器，共用上述固定 FSMN-VAD。三份文件的 revision、大小、SHA-256 见[当前集成契约](../docs/fun-asr-nano.md)，均保留 Apache-2.0。既有 `sensevoice-model-LICENSE` 文件名仅为历史命名，内容是两者共用的 Apache-2.0 标准许可，安装时仍复制为模型目录 LICENSE。

不复制或修改推理算法，不引入 FunASR Python/PyTorch。新程序来源 JSON 标明 `model=Fun-ASR-Nano`，生成资源目录移除旧 SenseVoice helper，原用户安装和旧文案保留。下文 SenseVoice/CUDA 为历史来源与验收记录，文件名及旧构建命令不代表当前可执行入口。

## SenseVoiceSmall 本地 CPU 转录（2026-10-23，历史记录）

用户批准单个已下载视频提取文案、复制和 TXT 导出，选择官方 GGUF/C++ CPU 路径，复用现有 sidecar 编排，不引入 FunASR Python、PyTorch、CUDA 或服务端。`scripts/build_sensevoice.py` 下载以下固定源码，完整核对归档 SHA-256，只构建 `llama-funasr-sensevoice`；不修改识别算法。

| 来源 | 固定提交 | 源码 tar.gz SHA-256 | 许可 |
| --- | --- | --- | --- |
| [modelscope/FunASR runtime/llama.cpp](https://github.com/modelscope/FunASR/tree/fbbfe01800a8c4c9103cb2150fb1a5d06cc64758/runtime/llama.cpp) | `fbbfe01800a8c4c9103cb2150fb1a5d06cc64758` | `be42f6f6fa86468c3db8ea93b7c1ae80e7389a49be9df989f82a5c59577290bd` | MIT |
| [ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp/tree/803b7fcae893e9caaee3921779628fef83ac0965) | `803b7fcae893e9caaee3921779628fef83ac0965` | `8fe8528e89d8fca8c8b81696efa83ff82c996bf84d17be65e884c1ceae35351e` | MIT |

固定 CMake 参数禁用 GGML_NATIVE、AVX/AVX2/FMA/F16C/SSE42/BMI2、CUDA、Metal、Vulkan、OpenMP 和 llamafile，避免继承构建机指令集或引入 GPU/runtime 依赖。Windows 静态 MSVC runtime，加 `vendor/sensevoice-windows.manifest` 的 UTF-8 activeCodePage 元数据以支持中文路径；macOS helper 使用 deployment target 12.0 和 ad-hoc 签名（应用最低 macOS 13.3 不变）。输出 helper、源码修订 SOURCE.json、FunASR/llama.cpp MIT、miniaudio 的 public-domain/MIT-0 选择许可、模型 Apache-2.0 和冒烟 WAV 一起进入 PyInstaller 资源 `asr/`。模型权重不进安装包。

两个官方模型分别来自 [SenseVoiceSmall-GGUF](https://huggingface.co/FunAudioLLM/SenseVoiceSmall-GGUF/tree/90c1c61912018b70ada0fcc024ea24aca62f2e63) 和 [fsmn-vad-GGUF](https://huggingface.co/FunAudioLLM/fsmn-vad-GGUF/tree/6840bae4c5c92ee8c04faaf4db23dd0105098d7f)。官方模型卡均标 Apache-2.0；标准许可全文保存于 `sensevoice-model-LICENSE`，每次模型安装也复制到模型目录。固定文件大小/hash 已在 Windows 完整下载核对，详见 `python-engine/runtime_assets.py` 和[方案](../docs/sensevoice-transcription-plan.md)。升级须重新核对源代码/模型许可、query_tokens、CLI 契约和三平台真实推理，不能仅更新版本标签。

冒烟音频为 [FunAudioLLM/Fun-ASR-Nano-2512 的官方 example/zh.mp3](https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-2512/blob/272c57b82523ada6fd87095e955f8e29100979ab/example/zh.mp3) 前 3 秒，经 FFmpeg 转为 16 kHz 单声道 PCM16，来源模型卡标 Apache-2.0。原 MP3 SHA-256 `0e64de19e4ff9a02e682955c9112f32d2317cfdbb5bc2f3504664044c993f195`；入库 `python-engine/assets/sensevoice-smoke.wav` SHA-256 `ff683faa421d58af0c3f6c5831954ce0e2eb1e71da89196ec9b3d3eb5105cb6a`。它仅用于模型可加载、可推理检测，不代表语言/自然视频识别质量验收。

Rust 增加 `same-file 1.0.6` 直接依赖（原锁文件已有），通过原生文件身份拒绝 TXT 覆盖源视频及硬链接/符号链接别名；Windows 传递依赖 `winapi-util 0.1.11`。二者均 `MIT OR Unlicense`，选择 MIT，许可全文见 `transcription-licenses.txt`，随安装包放入 `licenses/transcription-licenses.txt`。未复制其源码实现。

## SenseVoice Windows CUDA 可选组件（2026-10-23，历史记录）

复用 [modelscope/FunASR v1.4.11](https://github.com/modelscope/FunASR/releases/tag/v1.4.11) 的 `funasr-llamacpp-windows-x64-cuda.zip` / `funasr-llamacpp-windows-x64-cuda-blackwell.zip`，分别为 sm_86 / sm_120、CUDA 13.2、静态 MSVC runtime；不自行改写推理算法。固定完整 ZIP 大小与 SHA-256 位于 `python-engine/cuda_runtime.py` 和 [CUDA 集成说明](../docs/cuda-acceleration.md)，升级需重新核对架构、DLL、许可证、离线加载、识别及取消/打包。标准包已完整下载计算哈希并在 RTX 4070 SUPER 推理；Blackwell 包仅核对官方元数据，物理硬件未测。

仅安装 SenseVoice helper、`cublas64_13.dll`、`cublasLt64_13.dll` 和 `NVIDIA-CUDA-LICENSE.txt`，原样保留 NVIDIA CUDA 再分发许可。FunASR Apache-2.0、llama.cpp MIT、miniaudio 双许可文本从当前内置资源一并复制；原生源码基线仍见上节，不引入完整 Python FunASR 或 CUDA Toolkit。模型沿用现有 Q8/VAD 来源和 Apache-2.0 许可，CUDA 组件不包含权重。

## audio.cpp Nano CUDA（2026-10-23，当前）

2026-10-25 补充：`scripts/build_asr.py` 为固定 FunASR CPU CLI 添加专用 `YINGQU_ASR_PROGRESS` stderr 通知并逐次刷新，仅报告分段阶段、实际窗口总数和完成数，不改推理、VAD、正文输出或许可。源码替换需精确匹配，重复构建幂等，SOURCE.json 标记 `progress_version=1`；上游提供等价完成回调后可移除。CUDA 继续复用固定 audio.cpp 的 `--log-file` 准备记录和批次结果文件，无原生修改。详见[真实分段进度](../docs/fun-asr-nano.md)。

复用 [0xShug0/audio.cpp v0.9.2-drama3-base-small](https://github.com/0xShug0/audio.cpp/releases/tag/v0.9.2-drama3-base-small)，固定模型、程序 ZIP 和 CUDA 12.4 DLL ZIP 的 URL、大小、SHA-256 见 [CUDA 集成](../docs/cuda-acceleration.md) 与 `python-engine/nano_cuda.py`。只提取 CLI、所需 DLL、LICENSE，不运行 server/下载管理器。模型 Apache-2.0，原生库许可随包保留；CUDA EULA 本地副本 `vendor/nvidia-cuda-12.4-EULA.html` 随 native 资源和 CUDA 安装保存。VAD 复用已固定的 FunASR `llama-funasr-vad` 目标，不改写检测算法。无新增 Python ML 依赖。


## 关键帧 PNG 校验

WB-304 直接复用锁文件已有的 [image-rs/png 0.18.1](https://github.com/image-rs/image-png)，在现有 8 MiB / 1280 像素边界内验证 PNG 能实际解码并读至文件结束，避免只凭签名接收截断图片。`png = "0.18"` 沿用现有解析版本，不升级其他依赖；PNG 及其依赖许可全文见 `keyframes-licenses.txt`，随安装包放入 `licenses/keyframes-licenses.txt`。源码未复制，上游 Apache-2.0 / MIT 许可适配本项目。抽帧仍调用已有 FFmpeg，不分发新的媒体工具。
