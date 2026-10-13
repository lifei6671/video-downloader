# M1：yt-dlp 迁移验证

日期：2026-10-13。**双平台源码短链、Windows 打包长链均已下载通过；全新无 Python Windows 环境仍待验证；用户于 2026-10-13 批准移至 M3，并进入 M2。** 抖音直连 ACCESS_DENIED 已定位并修复，引擎版本 0.2.1。旧引擎结果单独保存在[历史记录](m1-legacy-validation.md)，不计入当前验收。

## 实现与依赖

- Python 使用固定 `yt-dlp==2026.8.19` 统一解析与媒体下载，协议 v2，每次预览/下载各自一个进程；下载重新解析并核对平台和作品 ID。
- Rust 管理进程、超时/取消、协议进度和最终文件提交；取消/失败先终止并等待子进程，再清理暂存目录。终端消息和进程正常退出都满足后，核对大小、容器头，再以不覆盖方式提交文件。
- 仅直接 HTTP(S) 单文件 MP4/WebM；拒绝 playlist、直播、分轨、分片，禁用插件、外部下载器、重试、断点续传和转码。
- 实际 yt-dlp urllib 网络路径检查目标及重定向；直连验证公共 DNS 地址并连接同一结果；平台 Cookie 为 HTTPS 域限定，不向 CDN 附加原始 Cookie。代理远端 DNS 不能提供相同的本机 IP 证明。
- 已删除 dtk 源码、补丁、旧 fixtures、独立试验脚本、Rust HTTP 下载器及 reqwest；Python 运行依赖仅 yt-dlp。来源与许可证见 [UPSTREAM](../vendor/UPSTREAM.md)。

M1 阶段尚无 Tauri UI、SQLite、队列、系统凭据存储或安装器；后续桌面实现见 [M2 验证记录](m2-validation.md)。M1 文件名仍为 `平台_作品ID.mp4/webm`。启动命令见 [README](../README.md)。

## 离线与打包

Windows x64，Python 3.12.13，Rust/Cargo 1.98.0，PyInstaller 6.22.3。

| 命令 / 场景 | 结果 |
| --- | --- |
| `cargo build --manifest-path src-tauri/Cargo.toml --bin m1-probe` | PASS |
| `cargo test --manifest-path src-tauri/Cargo.toml` | PASS，3 项实际测试 |
| `.venv/Scripts/python.exe -m pytest -q` | PASS，74 项，无跳过 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | PASS |
| PyInstaller onedir 构建 | PASS，包含 yt-dlp、Python、PyInstaller 与本项目许可证 |
| 原生探针启动打包引擎 | PASS：临时独立 cwd，PATH 仅 System32，移除 PYTHONPATH/PYTHONHOME/VIRTUAL_ENV |
| 全新未安装 Python 的 Windows 虚拟机 | NOT_RUN，当前环境无法提供该证据 |
| macOS Intel / Apple Silicon | NOT_RUN，当前无对应运行环境 |

离线覆盖输入与格式限制、元数据与身份核对、平台配置隔离、真实 yt-dlp 下载器经本地 fixture 的已知/未知长度、错误页、截断、重定向拒绝、同名不覆盖、协议版本/ID/重复终端/EOF/非零退出/敏感错误脱敏、启动超时、阻塞下载 stdin EOF、Windows 父进程异常退出及 Ctrl+Break 取消回收。fixture 媒体不是实际平台视频；真实可播放性见下一节。数据库恢复、并发隔离属于后续阶段，不作为本次已测试内容。

## 双平台实际验证

最终配置：**抖音直连（`proxy: null`），TikTok HTTP `127.0.0.1:3067`**；Cookie 从授权私有文件读取，UA 为 Edge 150。直连不会继承环境代理；显式 HTTP 代理不会被 NO_PROXY 或系统 bypass 改为直连，已增加实际 request director 的 HTTP/HTTPS 回归。

| 引擎与输入 | 抖音 7682077475388978298 | TikTok 7676209499589446945 |
| --- | --- | --- |
| 源码，双平台短链 | PASS：466,223 bytes | PASS：687,985 bytes |
| 打包，双平台长链 | PASS：466,223 bytes | PASS：687,985 bytes |

实际命令：`scripts/run_m1.py .local/douyin-fix-source-v2.json`（由 `.venv/Scripts/python.exe` 执行）；打包测试由原生 `m1-probe.exe` 直接启动 `python-engine/dist/video-engine/video-engine.exe`，配置与 Cookie 经 stdin 输入。两次组合测试退出码均为 0，两平台均报告 completed。

输出目录：`.local/douyin-fix-source-v2/`、`.local/douyin-fix-packaged/`。抖音文件 SHA-256 均为 `29c08afc224492d400a3dac16039cad34e1cb11632da19d4b4e7042ce208086a`，HEVC 720×1280、AAC、约 6.85 秒。打包输出经过 FFmpeg `-v error -xerror -i <file> -f null -` 完整解码退出 0，源码输出哈希相同。TikTok 文件 SHA-256 均为 `27a37a403642d5ceae29a38576d776e7e2cb67840df794017cccd0c144be4c30`，与已完成 FFmpeg 全量解码（退出 0）的本轮输出相同。视频 HEVC 1024×576、音频 AAC、约 18.85 秒。FFmpeg 仅用于外部验收，不是运行依赖。

### 抖音原因定位与修复

此前 ACCESS_DENIED 是真实失败记录；更新 Cookie、直连和 impersonation 均未修复。此次同一 Cookie/UA、明确直连进行对照：

| 请求 | 官方原始 transport | 自有 guarded transport |
| --- | --- | --- |
| 固定版原请求，仅 aweme_id | HTTP 403：Uifid Not Found | HTTP 403：Uifid Not Found |
| 原请求加 Cookie 中 UIFID | HTTP 403：Signature Not Found | 未单独重复该中间步骤 |
| 加入已批准的详情签名 | 未单独重复 | HTTP 200、status_code=0、目标作品 ID 一致 |

第一处根因是上游 DouyinIE 请求构造不适配平台当前详情接口，统一“Fresh cookies”提示掩盖了实际拒绝原因，不是本应用错误开启代理或丢失 Cookie。桌面网页能返回 200 不能替代详情 API 成功。

用户明确批准最小签名例外后，自有 `douyin.py` 仅对单视频详情端点补充 UIFID 与 x-secsdk-web-signature，沿用 yt-dlp 提取器其余逻辑。签名研究来源、编码及移除条件见 [UPSTREAM](../vendor/UPSTREAM.md)。没有恢复 dtk 或新增依赖。

第二处是本应用适配问题：详情恢复后真实媒体主机 `v26-default.365yg.com` 不在抖音域清单，触发 TargetError，被 RequestError 包装而误报 NETWORK_ERROR。现将 `365yg.com` 限定加入抖音目标范围，并正确分类包装后的目标拒绝，保留 DNS 和重定向校验。新增 8 项测试覆盖签名向量、Cookie 缺失、端点限制、注册隔离、CDN 平台边界及错误分类。

## 未通过项

M1 的双平台实际下载条件已满足；全新无 Python Windows 环境仍未验证，macOS 仍为 NOT_RUN。用户已批准进入 M2；上述全新 Windows 验收保留为 M3 待验证。旧忽略缓存的删除被自动审批以 `blocked by policy` 拒绝，缓存不参与源码导入、构建或分发，旧生产依赖已移除。
