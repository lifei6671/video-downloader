# macOS 适配验证

日期：2026-10-14。基线：`43d7021` 加本次未提交改动。环境：Apple Silicon arm64、macOS 27.0.1、Xcode Command Line Tools、Rust 1.99.0、Node 22.22.0、Python 3.12.14、PyInstaller 6.22.3、yt-dlp 2026.08.19。

结论：Apple Silicon 本机编译、离线核心行为、原生应用启动、DMG 搬移启动，以及双平台原生 GUI 的预览、下载进度、本地封面、完成记录和正常退出后的重启恢复通过。完整 M3 其他场景尚未全部验收，不能据此宣称 Intel 或 macOS 13.3 实机通过。2026-10-14 用户明确不要求 Developer ID / 证书，本项目 macOS 采用本地 ad-hoc 签名，不将证书或公证设为本次验收阻塞项。

## 本次改动

- 将开发依赖锁文件改为 `uv pip compile --universal` 生成，补齐 macOS 必需的 `macholib`，保留原有依赖版本，并为 Windows 专属依赖添加环境标记。CI / Release 安装后执行 `pip check`。
- 新增 `tauri.macos.conf.json`，声明最低系统版本 13.3。原包默认声明 10.13，但当前引擎 Mach-O 最低版本为 11.0，现有 Tailwind 4 要求 Safari/WebKit 16.4；采用 13.3 作为打包下限，旧系统实测仍待完成。依据：[Tailwind](https://tailwindcss.com/docs/compatibility)、[WebKit](https://webkit.org/blog/13966/webkit-features-in-safari-16-4/)。
- 取消测试在 Windows 使用 CTRL_BREAK、macOS 使用 SIGINT；不再跳过 POSIX。修正 Ready 重启测试的线程退出等待，不改变生产状态或退出流程。

## 构建与离线结果

| 验证 | 命令 / 证据 | 结果 |
| --- | --- | --- |
| 依赖完整性 | `uv pip check --python .venv/bin/python` | PASS，11 个包兼容 |
| Rust 探针 | `cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe` | PASS |
| Rust 行为 | `cargo test --locked --manifest-path src-tauri/Cargo.toml` | PASS，23 项；含取消、队列、文件提交、历史和重启 |
| Python / bridge 集成 | `.venv/bin/python -m pytest -q` | PASS，93 项，0 跳过；含 SIGINT、父进程退出、阻塞网络 EOF 回收 |
| Rust 门禁 | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`、`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | PASS |
| 前端 | `npm run build` | PASS；Vite 报既有 `use client` / sourcemap 警告 |
| 引擎打包 | `.venv/bin/python -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec` | PASS，arm64 onedir |
| 引擎启动 | `scripts/check_packaged_engine.py` 分别检查独立 onedir、最终 `.app` 内引擎、DMG 搬移副本内引擎 | PASS，协议 2 / 引擎 0.2.1 / yt-dlp 2026.08.19；隔离 cwd 与开发 PATH |
| Tauri 安装包 | `APPLE_SIGNING_IDENTITY=- npm run tauri -- build --features desktop --config src-tauri/tauri.release.conf.json --bundles app,dmg -- --locked` | PASS |
| 签名与最低版本 | `codesign --verify --deep --strict`、逐个 `codesign --verify --strict` / `otool -l`、`plutil` | PASS，4 个 Mach-O 均为 minOS 11.0，Info.plist 声明 13.3；仅 ad-hoc |
| DMG / 安装路径 | `hdiutil attach -readonly -nobrowse`、`ditto`、`hdiutil detach` | PASS，映像校验、复制到中文空格路径、卸载成功 |

初始失败保留：原锁文件安装后 PyInstaller 报 `ModuleNotFoundError: No module named 'macholib'`；`uv pip check` 同样检出缺失。初次 Rust 测试为 22 PASS / 1 FAIL，Ready 重启测试报 `APP_ALREADY_RUNNING`，原因是 Ready 已发布但解析线程尚未释放 Core 引用。等待该线程真正退出后重开数据库，23 项全部通过；未增加睡眠、重试或弱化恢复断言。

## 原生应用结果

使用最终 `.app` 和从 DMG 复制的 `.local/macOS 安装验证/映取.app`，通过原生 UI 工具观察与交互；不使用浏览器 mock 替代 Tauri。

- PASS：亮色首页 / 设置页可见，系统原生标题栏、下载目录为本机 `Downloads/Yingqu`，两平台初始 Cookie 均未配置。
- PASS：⌘V 粘贴、无效分享文案解析及持续错误展示；搬移副本同样能通过随包引擎得到 `INVALID_INPUT`，未报 `ENGINE_UNAVAILABLE`。搬移副本从自身目录启动，PATH 为 `/usr/bin:/bin`。
- PASS：原生目录选择器打开 / 取消，取消后回显下载目录保持一致。
- PASS：重复启动搬移副本返回 0；通过原进程实际映射文件路径及 PID 核对，仍只有一个应用进程。
- PASS：⌘Q 正常退出，搬移副本退出码 0；本次临时安装副本与挂载目录已清理。
- NOT_RUN：托盘菜单和真实 Dock 点击的独立交互验收、进行中下载时托盘退出、Finder 定位成功视频、Keychain 代理凭据回读、三个设计视口的完整视觉验收。此次没有修改前端布局或控件。

## 用户作品在线验证

样本：用户提供的抖音分享文案及 `https://v.douyin.com/RY8wkmtDRGM/`，赛博食录的“一年89万吨！为什么欧美人爱偷奶酪？”。

首次在未配置 Cookie 的原生 `.app` 实际提交分享文案，先显示解析中，随后显示“平台拒绝访问，请检查 Cookie 或稍后重试”，任务结果为 `ACCESS_DENIED`。此结果为 FAIL（当时访问条件不足），保留原始失败，不将该 UI 文案当作远端返回 HTTP 403 的证据。现有用户批准的详情请求补丁要求 Cookie 中存在 UIFID / UIFID_TEMP，不自动生成身份。

随后用户授权读取本机 `tests/douyin.txt`，指定 Edge 150 UA。配置仅在忽略目录临时保存 Cookie 文件路径；运行器在内存读取 Cookie 后经 stdin 传递，未输出 Cookie、签名媒体 URL，也未写入应用设置数据库。

```sh
.venv/bin/python scripts/run_m1.py .local/macos-online.json --probe src-tauri/target/release/bundle/macos/映取.app/Contents/MacOS/m1-probe --engine src-tauri/target/release/bundle/macos/映取.app/Contents/Resources/video-engine/video-engine
```

结果：PASS，探针退出码 0，作品身份 `douyin / 7696105473535135018`；解析后下载进程重新解析并核对身份，进度达到 44,144,781 字节，Rust 校验并提交 `.local/macos-online-download/douyin_7696105473535135018.mp4`。回读文件大小与报告一致，MP4 `ftyp` 校验通过，输出目录无 `.part` 或暂存残留。SHA-256：`aa33c044b412374e94620e01a92a66a5adbfe0d45f72080b48a7c5e52d1ef7ee`。临时配置已删除，测试视频保留供用户检查。播放 / 解码为 NOT_RUN，本机无 ffprobe，未安装额外工具。

以上阶段是最终安装包内原生探针与生产引擎的真实网络下载；该阶段没有通过 GUI 保存 Cookie 或点击开始下载，GUI 完成记录 / 本地封面当时为 NOT_RUN。随后 GUI 验证见下文，保留探针与 GUI 两层证据。

### TikTok

用户随后提供 `https://vt.tiktok.com/ZSg1WSvdL/`、授权的 `tests/TikTok.txt` 和同一 Edge 150 UA。先按直连运行，实际结果为 FAIL / `INVALID_INPUT`。只读诊断发现 `vt.tiktok.com` 与 `www.tiktok.com` 的系统 DNS 同时返回 `fc00::` 私网 IPv6，既有直连公共地址校验拒绝这些结果；未放宽校验或修改系统网络配置。

确认项目验证入口默认 SOCKS5 `127.0.0.1:3067` 可连接后，显式修改本次测试 profile 的 proxy 再运行，结果为 PASS，退出码 0，作品身份 `tiktok / 7664079467140173086`。解析 / 重新提取核对身份 / 真实字节进度 / Rust 文件提交均完成，最终视频 `.local/macos-tiktok-download/tiktok_7664079467140173086.mp4` 为 4,805,731 字节，MP4 `ftyp` 与回读大小通过，输出目录无 `.part` 或暂存残留。SHA-256：`41984c9b638e0b18a14705432af0516c2a2cf13c9c34dc79ef04891b713e3d40`。

运行命令与抖音相同，只将临时配置换为 `.local/macos-tiktok.json`；配置仅包含授权 Cookie 文件路径，Cookie 在内存读取并经 stdin 传递。临时配置已删除，测试视频保留。直连失败与代理成功分别保留，代理成功不证明本机 TikTok 直连可用；该探针阶段 GUI 完成记录 / 封面及播放解码当时为 NOT_RUN。

## 双平台原生 GUI 下载闭环

用户授权继续使用上述 Cookie、UA 和链接，明确指定 TikTok `socks5://127.0.0.1:3067`。本阶段使用最终 Release `.app` 的实际窗口，未使用探针或浏览器 mock 代替 GUI 操作，未修改生产代码。

| 检查 | 抖音 | TikTok |
| --- | --- | --- |
| GUI 配置保存 | PASS，Cookie 已配置、直连、指定 Edge 150 UA | PASS，Cookie 已配置、指定 Edge 150 UA、SOCKS5 3067 |
| 预览 | PASS，作品标题、`@saiboshilu`、10:28、封面和下载按钮 | PASS，`e2`、`@snasothea`、0:50、封面和下载按钮 |
| GUI 点击下载 / 进度 | PASS，先显示未知总量；观察到 74%、31.5 / 42.1 MB、5.8 MB/s | PASS，先显示未知总量；截图观察到 0%、1.0 KB / 4.6 MB、184.4 KB/s，随后完成；未截取更多中间百分比 |
| 完成记录 | PASS，2026/10/14 10:54:44，42.1 MB | PASS，2026/10/14 10:55:53，4.6 MB |
| 视频文件回读 | PASS，44,144,781 字节，MP4 头和 SHA-256 与探针产物一致 | PASS，4,805,731 字节，MP4 头和 SHA-256 与探针产物一致 |
| 本地封面 | PASS，`.cover.jpg` 9,965 字节，图片头合法，完成列表可见，无 cover_error | PASS，`.cover.jpg` 138,649 字节，图片头合法，完成列表可见，无 cover_error |
| 正常退出后重启 | PASS，相同任务 ID、状态、文件路径、大小和封面恢复 | PASS，相同任务 ID、状态、文件路径、大小和封面恢复 |

GUI 生成的任务：

- 抖音：`d2fe89eb-7bb7-4a85-804b-f4426e04e71f`，视频为 `~/Downloads/Yingqu/douyin_7696105473535135018_d2fe89eb-7bb7-4a85-804b-f4426e04e71f.mp4`。
- TikTok：`4115df06-98bc-4163-b8de-41499492d460`，视频为 `~/Downloads/Yingqu/tiktok_7664079467140173086_4115df06-98bc-4163-b8de-41499492d460.mp4`。

操作全过程通过原生 AX 状态和截图观察。两平台均通过设置页“保存配置”；对应用 SQLite 仅做只读核对，确认 Cookie 与用户文件逐字一致、UA 与指定值一致、代理隔离正确，未直接写数据库。正常 ⌘Q 退出后核对旧 PID 86654 已退出；重新启动得到新 PID 91184，“已完成”页恢复两条记录、大小、路径和封面，无“文件缺失”。设置页也恢复 Cookie 已配置、两平台 UA、抖音直连和 TikTok 已保存 SOCKS5 3067。

重启后只读核对视频哈希、封面字节数及任务 ID 均未变化，`task_staging` 为 0。持久化的 `video.cover_url` 为 null，列表根据 `cover_path` 经 `get_task_cover` 读取本地图片，不依赖重新获取远端封面。脱敏文件读回证据保存在忽略目录 `.local/macos-gui-readback.json`，不含 Cookie。

本次验证正常完成后的重启恢复；下载中异常崩溃 / 断电恢复、播放解码、其他托盘 / Dock 场景仍为 NOT_RUN。Cookie 按授权通过 GUI 保存于本机应用 SQLite，下载视频和封面保留，应用留在两条完成记录页面。

验证工具隐私记录：首次选择 Cookie 文件时，系统文件选择器的自动预览意外把 Cookie 原文带入工具输出。随后关闭原文输出，并对设置回显进行脱敏；Cookie 文本编辑器文档已关闭，源码及本验收文档不记录 Cookie 内容。此工具操作问题不计作产品下载失败。

## 2026-10-14 预览格式信息与音频调查

预览白名单增加所选格式的 `width`、`height`、`filesize`、`filesize_approx`，不返回其他格式或签名媒体 URL。界面显示尺寸，大小依次采用完成文件字节数、实际下载总量、格式精确大小、标“约”的估算；缺失显示“—”。旧记录不补请求，缺失尺寸仍显示未知。预览信息不改变进度总量或选格式规则。

- `tests/test_engine.py` 与 `tests/test_probe.py`：69 passed；覆盖所选格式而非其他格式、未知及非法数字、媒体 URL 不泄漏和进程协议。
- `npm run build`：PASS；现有 shadcn/Radix `use client` 与 sourcemap 警告仍存在。
- PyInstaller 和 macOS ad-hoc `.app` / `.dmg` 重新构建：PASS；`codesign --verify --deep --strict`：PASS。日志 `.local/metadata-engine-build.log` / `.local/metadata-macos-build.log`。
- 更新后的 Release 原生 GUI：抖音显示 `1280 × 720 / 42.1 MB`，TikTok 显示 `1080 × 1920 / 4.6 MB`，两者封面、作者和时长仍可见。通过 Tab / Return 提交解析，重复下载提示选择“暂不下载”，焦点返回下载按钮；未重复下载文件。
- 1440 × 900 与 1280 × 800 窗口（含原生标题栏）截图核对：字段无裁切。1024 × 680：NOT_RUN，当前 UI 自动化拖动未成功得到该尺寸，不将拖动尝试计作验证通过。
- 只读 SQLite 回读：新解析任务保留 `width` / `height` / `filesize`，`bytes=0`、`total_bytes=null`，预览大小未伪造下载进度。旧完成记录无需迁移。

音频调查使用用户授权的相同 Cookie / UA / 链接，抖音直连、TikTok SOCKS5 3067。仅在独立诊断进程内观察上游解析，不修改提取器，不持久化音频地址或凭据。沿用生产 GuardedRH 读取 32 字节后关闭响应：

| 样本 | 平台音轨 | 独立音频响应 | 边界 |
| --- | --- | --- | --- |
| 抖音 `7696105473535135018` | `@赛博食录创作的原声`，作者赛博食录，628 秒，原声标识 true | HTTP 200，audio/mpeg，ID3 头，Content-Length 25,142,472 | 原始详情 `music.play_url` 提供地址；当前 yt-dlp formats 未列出独立音频 |
| TikTok `7664079467140173086` | `original sound`，作者 Z1，50 秒 | HTTP 200，audio/mpeg，ID3 头，Content-Length 806,706 | 当前 yt-dlp 已列出 audio / mp3 格式 |

这些是作品音轨，不能证明是去除人声的纯背景音乐。完整音频下载、解码播放和视频音频提取均 NOT_RUN；本次未新增音频下载功能或 FFmpeg。上游字段来源见 [固定版本 TikTok / Douyin 提取器](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/tiktok.py)。脱敏诊断结果位于忽略目录 `.local/format-audio-diagnostic.jsonl`。

## 2026-10-14 抖音纯图片引擎能力调查

用户提供 `https://v.douyin.com/xhP440SkGa4/`，要求底层支持时接入。沿用已授权 Cookie / UA 与直连，结果如下：

- 分享短链展开为 `https://www.douyin.com/note/7687900413543421811`；生产 bridge 返回 `UNSUPPORTED_CONTENT` / input。
- 当前 yt-dlp 2026.08.19 的 Douyin 提取器只匹配 `/video/<id>`，该 `/note/` 地址 `suitable=False`；没有把图片列表转换为图集解析或下载结果的分支。核对上游当前 master 的 Douyin 提取器仍采用相同路径匹配和视频解析入口，未升级依赖。
- 独立诊断将相同作品 ID 交给既有详情提取入口：HTTP JSON `status_code=0`，`aweme_detail=null`，包含 `filter_detail`；上游将此映射为 Fresh cookies / `ACCESS_DENIED`。同配置的视频对照 `7696105473535135018` 返回有效详情并解析成功，因此不据此判定 Cookie 失效，也不宣称已取得图集图片。
- 用户的“底层支持时接入”条件尚不成立，本轮未修改生产代码。真正接入需要新增图文提取适配、图片列表协议及多文件保存/记录能力，不能仅放开 `/note/` 路径。

脱敏调查证据：忽略目录 `.local/douyin-images-diagnostic.jsonl`、`.local/douyin-images-upstream.jsonl`、`.local/douyin-images-api-check.jsonl`。原始响应、凭据、签名地址均未写入这些文件。源码依据：[固定版本提取器](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/tiktok.py)、[上游当前提取器](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/tiktok.py)。

## 产物与验收边界

- 应用：`src-tauri/target/release/bundle/macos/映取.app`。
- 映像：`src-tauri/target/release/bundle/dmg/映取_0.1.0_aarch64.dmg`。
- 本机最终构建日志：忽略目录 `.local/macos-build.log`。
- macOS Intel、macOS 13.3 / 其他旧系统、新 Windows 回归、GitHub runner 实际结果：NOT_RUN。本机离线数据与包搬移启动不替代全新无 Python 环境验收。
- Apple Developer ID / 证书、公证：NOT_REQUIRED（2026-10-14 用户决定，本次使用 ad-hoc）。下载后的 quarantine / Gatekeeper：NOT_RUN，不将本地启动描述为该场景已通过。

可复现命令见 [开发文档](development.md)。历史 M1/M2 证据保持原样，本记录仅补充本次 macOS 结果。
