# M1 核心链路验证

补充（2026-10-13）：独立 [yt-dlp 试验](ytdlp-validation.md) 已下载同一个 TikTok 样本并完整解码通过。下文为原 Rust/Python 链路的结果，仍保留其媒体 403 和 M1 未验收结论；尚未将 yt-dlp 接入该链路。

日期：2026-10-13。状态：**实现可运行，M1 未验收**。抖音解析与下载通过；TikTok 解析通过、媒体请求 HTTP 403，尚不能声称双平台下载跑通。

## 实现范围

- 固定上游 `v5.1.3` / `31d56df1462e99f24621eb2c6f060af183bf24fd`，仅两个包初始化补丁，不改签名或 parser 算法。
- Python 常驻 JSONL 引擎：分享文案提取、长/短链、签名详情请求、响应分类、媒体提取；平台 Cookie/代理/UA 独立。
- Rust 探针：版本/提交握手、消息边界、顺序调用、媒体域名/重定向检查、流式临时文件、进度、完成重命名与不覆盖、Ctrl+C 回收进程。
- Windows PyInstaller onedir 打包。没有 FastAPI、数据库或监听端口。

M1 不包含 UI、任务队列、SQLite、系统凭据存储、安装器。默认并发 2（可选 1–3）与二次确认删除本地文件已同步技术方案和 AGENTS，M2 才实现。探针文件名为 `平台_作品ID`，以新输出目录重复运行，尚非产品文件命名策略。Rust 完成判定使用 HTTP 状态、类型、文件头及已知 Content-Length；全量解码仅为本次外部验收，不是运行依赖。

## 环境与命令

Windows 11 x64，Rust/Cargo 1.98.0，Python 3.12.13，wreq 0.13.0，PyInstaller 6.22.3。精确依赖见 Cargo.lock 和 Python 锁文件。初始化、运行及打包命令见[README](../README.md)。

| 验证 | 结果 |
| --- | --- |
| `cargo build --manifest-path src-tauri/Cargo.toml --bin m1-probe` | PASS |
| `cargo test --manifest-path src-tauri/Cargo.toml` | PASS，5 项有实际断言的测试 |
| `.venv/Scripts/python.exe -m pytest -q` | PASS，50 项测试 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | PASS |
| PyInstaller spec 构建 | PASS |
| 打包引擎独立工作目录启动 | PASS，TEMP 工作目录、PATH 仅 System32、移除 PYTHONPATH/PYTHONHOME/VIRTUAL_ENV，由原生探针启动 |
| 全新无 Python 的 Windows 虚拟机 / 完整 Tauri 安装包 | NOT_RUN |
| macOS Intel / Apple Silicon | NOT_RUN |

离线测试包括上游 X-Bogus golden、A-Bogus 三个浏览器样本的 50 字段及 decode 比较、正常/删除/私密/图集/风控 replay、无效输入、签名 query/header 保留、无服务端依赖导入、域名清单一致性、JSONL 多请求/非法消息、子进程错误握手/EOF/错 ID、已知及未知长度下载、不覆盖、错误页面/截断拒绝与取消清理。

## 真实平台证据

两平台均使用用户指定 HTTP 代理 `127.0.0.1:3067`。Cookie 从用户授权本地私有文件读取，内容未进入日志或文档。抖音使用 Edge 150 UA；TikTok 分别验证默认 Chrome 130 无 Cookie、Edge 150 加用户 Cookie。

| 平台 | 作品 ID | 短链 | 长链 | Rust 保存 |
| --- | --- | --- | --- | --- |
| 抖音 | 7682077475388978298 | PASS：`v.douyin.com/YdewKHCl9D4/` | PASS | PASS，1,106,056 bytes，MP4 |
| TikTok | 7676209499589446945 | PASS：`vt.tiktok.com/ZSbEe5Vug/` | PASS | FAIL：`MEDIA_HTTP_403` |

源码引擎同一进程先处理抖音短链再处理 TikTok 短链；打包引擎同一进程先处理抖音长链再处理 TikTok 长链。打包引擎另用 TikTok Cookie/Edge UA 重试短链，结果仍为媒体 403。

抖音源码输出：`.local/m1-source/douyin_7682077475388978298.mp4`；打包输出：`.local/m1-packaged/douyin_7682077475388978298.mp4`。源码输出 SHA-256：`907082944e98b1dff934a00e04ad8974401ecfcde1eee27fb59cf2946c92220b`。FFmpeg `-v error -xerror -i <文件> -f null -` 全量解码退出 0。FFmpeg 通过临时 uv/imageio-ffmpeg 验证环境使用，未增加产品运行依赖。

TikTok 排查：详情成功返回视频信息；主媒体、下载地址和 bitrate streams 在当前出口均 403；httpx 和相同 wreq 浏览器指纹请求也未通过；用户 Cookie/Edge UA 重试、仅针对平台媒体主机附加 Cookie、去除 Referer、Range 探针均 403。未保存媒体签名 URL、Cookie 或响应原文。现有证据不能确定是出口、平台会话约束还是上游地址适配问题；没有更改签名、猜测替代 CDN 或把解析成功标为下载成功。

后续需获得能实际播放/下载的 TikTok 请求条件，定位 403 后补齐 TikTok 下载与解码证据，才能通过 M1。M1 尚未通过，不进入 M2。
