# yt-dlp TikTok 下载试验

> 历史记录：以下命令对应迁移前独立试验，`scripts/run_ytdlp.py` 已退役。当前统一入口为 `scripts/run_m1.py`，新链路结果见 [M1 验证](m1-validation.md)。

日期：2026-10-13，Windows x64。结论：**同一个此前媒体 403 的 TikTok 样本，通过 yt-dlp 下载成功，完整解码通过。**

## 范围与版本

- 官方上游：[yt-dlp/yt-dlp](https://github.com/yt-dlp/yt-dlp)，固定 PyPI 版本 `2026.8.19`（运行时显示 `2026.08.19`），与本次查询的最新稳定发布相同。
- `scripts/run_ytdlp.py` 为独立开发试验，直接使用官方 `YoutubeDL`；不修改签名、提取器或媒体地址，不引入自动双引擎回退。
- 使用 PEP 723 脚本依赖及 uv 隔离缓存，不改 M1 依赖锁、Rust 下载器或 Python bridge。抖音仍使用原链路。
- 不将此次 Python 下载成功等同于 Rust 媒体下载通过；生产引擎接入、打包和双平台 M1 验收尚未完成。

## 在线验证

样本：`https://vt.tiktok.com/ZSbEe5Vug/`，作品 ID `7676209499589446945`。读取既有本地私有 Cookie，使用既有 Edge 150 UA。未输出 Cookie、签名媒体 URL 或平台响应原文。

| 验证 | 结果 |
| --- | --- |
| SOCKS5 `127.0.0.1:3067`，`uv run --no-project --python .venv/Scripts/python.exe scripts/run_ytdlp.py` | PASS，退出 0，687,985 bytes |
| HTTP `127.0.0.1:3067`，同命令加 `--proxy http://127.0.0.1:3067` | PASS，退出 0，687,985 bytes |
| FFprobe 检查首个文件 | PASS，MP4，HEVC 1024×576，AAC，18.852993 秒 |
| FFmpeg `-v error -xerror -i <首个文件> -f null -` | PASS，完整音视频解码退出 0 |
| 入口及原运行器回归测试 | PASS，16 项；命令见 README |
| Rust 经 yt-dlp 解析后下载 / 打包 sidecar | NOT_RUN，本试验未接入 |
| macOS Intel / Apple Silicon | NOT_RUN |

SOCKS5 输出：`.local/ytdlp-20261013-121003-104165/tiktok_7676209499589446945.mp4`。

HTTP 输出：`.local/ytdlp-20261013-121111-077353/tiktok_7676209499589446945.mp4`。

首个文件 SHA-256：`27a37a403642d5ceae29a38576d776e7e2cb67840df794017cccd0c144be4c30`。

FFmpeg/FFprobe 使用已有 uv 缓存中的验证工具，没有增加产品运行依赖。

初次试跑因适配脚本误用了短链提取器类名而报告 `No suitable extractor`；已改为官方注册名 `vm.tiktok`，并增加真实注册表回归断言。该初次失败不是平台下载失败。

## 结论边界

这证明 yt-dlp 在当前样本、Cookie 与网络条件下可完成下载；不证明所有 TikTok 视频可用、凭据登录态有效或原上游 403 的根因已查明。下一阶段接入仍须保留 Rust 状态、下载取消与文件处理契约；本次没有开展该架构迁移。
