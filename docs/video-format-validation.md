# 视频规格选择与抖音 4K 验收

2026-10-14，macOS Apple Silicon。引擎 0.5.0、协议 v3，yt-dlp 2026.08.19、DTK 5.1.3。包含解析缓存、4K 修复和视频规格选择；验证未覆盖 `/Applications/映取.app`。

## 根因与修正

测试分享链接 `https://v.douyin.com/DeEhd_o_OZs/`，作品 `7696335899718697330`。

1. 原详情请求只有 aweme_id 和已有签名字段，返回最高播放地址为 720 × 1280 / 2238438 字节；父级 video.width/height 虽为 2160 × 3840，不能据此声称所下载文件是 4K。
2. 加入 `aid=6383`、`channel=channel_pc_web`、`detail_list=1` 后，平台返回含 4K 的完整码率列表。参数来源核对 [DownVid 抖音解析器](https://github.com/yxxbc/downvid/blob/main/electron/parsers/douyin.ts)，没有接入其浏览器或网络代码。
3. yt-dlp 的 URL-key 解析漏填该作品 `bytevc1_4k` 地址的尺寸，原排序仍选择较小视频；Douyin 子类按每个实际 play_addr 的尺寸补齐格式元数据，选择器按像素数、帧率、码率排序。
4. 首次真实 4K GUI 下载返回 403，失败记录 `a5563763-479c-42ea-ad77-121316a99a96` 保留。定位到媒体请求缺少根 Referer；同一 CDN 的受控 Range 请求无根 Referer 返回 403，带 `https://www.douyin.com/` 返回 206。下载显式补该头后成功，Cookie 域及网络边界没有修改。

## 最终行为

- 视频预览使用现有 shadcn/ui ToggleGroup 展示真实分辨率、容器、编码、码率与大小；本作品 15 个规格，默认 4K / 6.0 MB。相同规格的 CDN 镜像合并，不显示水印、DRM 或 bytevc2。
- 文件大小采用实际上游值；未知显示“大小未知”，只有上游估算值才标“约”。选择后顶部尺寸/大小同步更新。
- Rust 仅接受任务候选中的规格 ID，保存到任务元数据；ready 状态允许选择，排队/下载后不能改。SQLite 缓存保存完整白名单候选，不保存媒体 URL、签名或请求头。
- 缓存仍保留默认选择；用户选择在任务中独立保存，重启与重试重新应用，不被缓存默认值覆盖。
- 下载重新提取并匹配稳定规格 ID；规格消失时明确 `FORMAT_UNAVAILABLE`，不自动降级、分轨合并或转码。完成记录保存实际字节数与新提取尺寸。
- 引擎版本升级使旧缓存失效，原成功记录与本地文件保留。

## 真实文件及 GUI

两次均从真实 Tauri 窗口开始下载，使用用户此前授权并保存的 Cookie、UA，抖音直连。

| 规格 | 任务 | 实际字节 | AVFoundation 实际尺寸 | 结果 |
| --- | --- | --- | --- | --- |
| H.265 MP4 / 4K | `2c66fbb2-0aa5-43f7-a36d-5158a91ab697` | 6290708，界面 6.0 MB | 编码及显示均 2160 × 3840 | PASS |
| H.265 MP4 / 720p | `316f44ef-2cbe-45ce-abb4-036f8cf19259` | 2238438，界面 2.1 MB | 编码及显示均 720 × 1280 | PASS |

720p 先选择、正常退出，再重启：顶部 720 × 1280、2.1 MB、对应单选值与预览封面均恢复，然后下载成功。再次解析同一链接命中 SQLite，新的预览默认 4K，下载得到上表 4K 文件。4K 任务窗口捕获 `0% 1.0 KB / 6.0 MB`，随后完成 Toast；720p 完成 Toast 也可见。

原 10 条完成记录保留，新增后 12 条；完成列表可见 6.0 MB 与 2.1 MB 两条新记录及封面。正常退出、重启后完成页仍显示这两条记录和文件路径。文件实际长度、SHA-256 与尺寸检查证据在 `.local/video-format-gui-files.json`；尺寸通过 macOS AVFoundation 直接读取，不依赖平台声明。

## 自动检查与包

| 命令/项目 | 结果 |
| --- | --- |
| `.venv/bin/python -m pytest -q` | PASS，127 项 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --features desktop --lib -- --test-threads=1` | PASS，31 项 |
| `npm run build` | PASS |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | PASS |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASS |
| PyInstaller、打包引擎握手 | PASS，0.5.0 |
| Tauri `.app` / `.dmg`、ad-hoc 签名 | PASS |

新增回归覆盖 4K 尺寸补齐、实际分辨率排序、CDN 镜像去重、签名变化后稳定匹配、所选规格缺失不降级、未知大小、水印/DRM 过滤、选择后重启恢复、选择后下载及重试保留。已有 DTK 测试样本补齐新增读取的 bitrate 字段后全套通过；初始失败未通过删除或跳过测试处理。

日志 `.local/video-format-python-tests.log`、`.local/video-format-rust-tests.log`、`.local/video-format-clippy.log`、`.local/video-format-macos-build.log`。本轮未重新进行 TikTok 或 DTK 的在线视频下载，相关离线下载/网络回归通过，不冒充双引擎双平台在线验收。

1440 × 900 真实窗口中的规格卡片、滚动选择、预览、完成及重启 PASS。原生拖拽未能改变窗口尺寸，1280 × 800 / 1024 × 680 本轮未取得有效窗口证据，记 UNAVAILABLE；键盘选择本轮 NOT_RUN。一次后续窗口读取失败，重置 CUA 绑定后恢复并完成历史页及重启验证。Windows / macOS Intel 本轮 NOT_RUN。

## TikTok 追加在线验收

2026-10-14 后续使用 `https://vt.tiktok.com/ZSgJMkMPn/` 完成 1080p 和 720p H.265 的真实下载、文件尺寸/字节核验、选择与完成记录重启恢复，均 PASS；具体规格、任务和边界见 [TikTok 视频规格在线验收](tiktok-video-format-validation.md)。这是后续独立证据，不改写本轮尚未运行 TikTok 的记录。

## 抖音音视频类型修正（2026-10-19）

用户批准仅修正规格分类并过滤当前不支持的类型，不提交、不引入抖音音频下载或分轨合并。起始工作区干净，HEAD 为 `ab314659d64e29ab73855e9ac008b4c91c1ee866`。

在自有 `DouyinIE` 调用上游 parser 后，按每个地址关联 `bit_rate` 与 `video_extra`，不使用顶层 `video.format` 统一定性。MP4 需要格式、有限正数的声道数和采样率证据一致；DASH 关联独立音轨时纠正 `acodec=none`。独立音轨及原声地址标为音频；未知、缺失及冲突不猜有声。选择器只放行抖音已确认的一体规格，解析及下载重提取使用同一规则，内部类型不进入公开规格或 SQLite。引擎同步为 0.7.2、协议 v3，现有缓存 revision 自动淘汰旧预览；历史文件保留。

### Windows 在线结果

使用用户本轮授权的 `tests/douyin.txt`，Cookie 只在内存/sidecar stdin 中传递，抖音直连。样本为 `https://v.douyin.com/hcgjYjKMphs/`，规范身份 `7664441562780702287`。

- 源码解析返回 11 个一体规格，逐项读取至多 256 KiB 媒体头，FFprobe 全部确认同时存在视频轨和音频轨。原始响应仍包含 DASH 档位，但不进入可下载列表。生产代码没有新增媒体头请求。
- 源码及重建后的 sidecar 均返回同一组 11 个稳定规格 ID，均以同一 ID 重新提取并完整下载 1080p H.265 MP4：9,436,229 字节，轨道为 HEVC 1920 × 1080 和 AAC 44,100 Hz 双声道。
- 两条路径均验证旧 DASH 规格 ID 返回 `FORMAT_UNAVAILABLE`，未创建媒体文件，没有自动切换到其他规格。
- Rust 探针使用重建后的 sidecar 完成解析、流式下载、文件校验和不覆盖提交。最终文件 `.local/douyin-specs-20261019/probe-downloads/douyin_7664441562780702287.mp4`；SHA-256 为 `16dee3994f96fafbf9bc3e4697f4bacdf706f8d57276fb20ec88ab4d0d59b8b7`，FFprobe 再次确认上述音视频轨。

脱敏证据 `.local/douyin-specs-20261019/source.json`、`packaged.json`、`probe-file.json` 只保留白名单规格、轨道、字节数和本地文件结果，不保存 Cookie 或签名媒体地址。在线命令：

```powershell
.venv/Scripts/python.exe scripts/run_m1.py .local/douyin-specs-20261019/probe.json --engine python-engine/dist/video-engine/video-engine.exe
```

### 本轮检查

| 检查 | 结果 |
| --- | --- |
| `.venv/Scripts/python.exe -m pytest -q tests/test_engine.py tests/test_probe.py tests/test_network.py tests/test_video_formats.py tests/test_douyin.py tests/test_youtube.py` | PASS，154 项 |
| `cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe` | PASS |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib parse_cache::tests -- --test-threads=1` | PASS，5 项，88 项不在本次筛选范围 |
| `npm run build` | PASS；已有 use-client、sourcemap 和包体积提示不作为通过依据 |
| PyInstaller onedir 重建及 `scripts/check_packaged_engine.py python-engine/dist/video-engine/video-engine.exe` | PASS，0.7.2，隔离 cwd/PATH 启动 |
| 浏览器预览 1440 × 900、1280 × 800、1024 × 680 | PASS，复用明确隔离的 `background-download.html?scenario=douyin` IPC fixture；说明可见、无横向溢出，Space 键选择可用 |
| 实现阶段 Tauri GUI、macOS、全新无 Python Windows 安装验收 | NOT_RUN；实现阶段为 Windows 源码/sidecar/Rust 探针和浏览器 fixture 验证；后续桌面实测见下节 |

回归直接复用真实 yt-dlp parser，覆盖 MP4/DASH 同分辨率、数字字符串与数字字段、音频/音乐地址、元数据缺失/冲突、URL 冲突、未知地址、旧规格不可用，以及解析到下载重提取的共同候选。初版数字字段校验未接受平台实际返回的字符串，在线验证报规格不支持；按实际响应修正并新增回归后上述检查通过，没有弱化未知类型策略。

### 后续 Windows Tauri GUI 验收（2026-10-19）

按用户要求执行 `npm run desktop`，重新编译并启动真实 `src-tauri/target/debug/yingqu.exe`，通过 Windows 原生窗口操作输入同一短链并点击解析。实际界面显示 yt-dlp、11 个规格及一体规格说明，默认选择 1920 × 1080 / H.265 / 9.0 MB；滚动核对全部规格。只读 SQLite 确认新任务 `56887bd7-2477-46f6-909b-4fb17b336bdc` 的 `cache_hit=false`，11 个规格 ID 与源码/打包引擎证据完全一致。

界面发现既有成功记录后，先关闭提示以检查规格，再点击“下载视频”并确认“重新下载”。下载完成后首页清空，下载任务页出现本次“已完成 · 9.0 MB”记录。该任务保留所选 ID `6892b0bfc9a8455981a73a2a`，新文件为 `F:\Yingqu\douyin_7664441562780702287_ebddd3d0-99a0-45cc-9078-cff992445fb9.mp4`，9,436,229 字节。FFprobe 确认 HEVC 1920 × 1080 + AAC 44,100 Hz 双声道，SHA-256 与上节文件相同。已有三条同作品任务均保留，既有成功视频仍在原路径，本次没有覆盖或删除旧文件。

GUI 验收 PASS。证据 `.local/douyin-specs-20261019/gui-ready.png`、`gui-specs-bottom.png`、`gui-completed.png`、对应 UIA 文本、`gui-resolved.json` 与 `gui-result.json`。没有声称逐帧观看或主观听音通过；本轮未重跑 macOS/全新无 Python Windows。已核对实现文件哈希与独立评审冻结目标一致，此次只追加验收记录，未修改实现、暂存或提交；按用户要求保留桌面程序运行。
