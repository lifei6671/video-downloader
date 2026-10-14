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
