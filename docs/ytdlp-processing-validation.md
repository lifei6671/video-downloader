# yt-dlp 调用与媒体数据处理专项（2026-10-21）

当前实现使用引擎 **0.7.5**、JSONL **v3**，固定 yt-dlp **2026.08.19**、DTK 5.1.3。未升级依赖，不修改签名算法、DTK vendor、平台域名/协议/端口/重定向策略。Rust 继续管理任务、队列、SQLite、最终文件校验与不覆盖提交。原专项验证使用 0.7.4，以下保留当时证据；新增 YouTube 设置见文末补充。

## 基线与参考

- 起始分支 `master`，HEAD `c4db99041011b25805b69804c53c279a2cd015bc`；只有用户已有的未跟踪 `output/`，本轮保持原样，没有提交或推送。
- Python 重点三文件基线 110 passed；前端构建通过，已有 use-client/sourcemap/包体积提示。
- 用户指定的 Rust 默认并行命令基线为 80 passed、18 项 Telegram 超时失败、2 ignored。项目 CI 已规定 `--test-threads=1`；最终按该方式执行，具体结果见下表。没有为并行测试超时修改 Telegram 实现。
- 只读取 [DownVid v1.5.0](https://github.com/Downvid/downvid/tree/v1.5.0)，发行标签指向 `05a14bb`。核对了用户列出的 Go 参数/测试/进度/格式文件、前端 dedupe 与 Electron 参数/格式文件。借鉴三类思路：下载变体身份、帧率显示归一化、原生 HTTP 分块。未采用其格式降级选择器、分辨率独占去重、估造大小、aria2 或独立 HTTP 下载器。
- 参数语义以本机锁定包及 [固定版 HttpFD](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/downloader/http.py)、[YouTube extractor](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/youtube/_video.py) 为依据。

## 问题与实现

| 范围 | 原因 | 当前行为 |
| --- | --- | --- |
| 重复下载 | Rust 与多处前端只比较平台和作品 ID，未核对磁盘文件 | 作品身份为 `platform + video_id`；视频变体另比较 `kind + selected_format_id`。只有其他 completed 记录且实际文件存在才提示；不同规格放行，失败/取消/排队不判重。Rust 在现有锁内实时判断，保持最终权威。 |
| 兼容判重 | 图集、Telegram 和旧记录不一定具有视频规格 | 图集按同作品同 kind；Telegram 按同消息原始视频；任一视频记录没有规格 ID 时按同作品同 kind 保守提示，文案明确历史规格未知。文件缺失允许重新下载。 |
| 前端规格切换 | 提示仅记任务 ID，提示和提交可能使用旧快照 | 共用变体判断，提示键含任务/类型/规格；提交前取最新本地任务，Rust 拒绝后刷新再判断。后端已接受同一任务后再次提交仍明确拒绝。 |
| YouTube 多音轨 | 从格式列表尾部取第一条音频，受列表顺序影响 | 优先有效 `language_preference`，再匹配作品原语言/语言，再使用默认音轨标记，之后按真实音频码率、声道、采样率及语言/format_id 确定性排序。不读取系统语言。 |
| 音轨真实调用链 | yt-dlp format callable 没有顶层语言；纯视频 `language=None` 还会覆盖原信息 | 在既有 match_filter 捕获本次作品语言，selector 与 public formats 使用同一份偏好。回归走真实 process_ie_result，再用返回 ID 下载及验证音轨缺失错误。 |
| 音视频兼容 | 只比较容器不足以保证无损合并 | MP4 配 M4A AAC/MP4A/ALAC；WebM 视频需对应可用编码，音频配 Opus/Vorbis WebM；两流均需 HTTP(S)、无 DRM/水印。没有兼容音频就不提供纯视频。FFmpeg 继续 stream copy。 |
| 规格数据 | 浮点帧率、编码别名、混合精确/估算大小 | 常见 NTSC 帧率归入 24/30/60 等显示和排序档位，保留真实不同编码/码率/规格。AVC/HEVC/AV1 显示规范名称，缺失编码仍未知。精确大小优先；合并任一流仅有估算时总和标估算；缺信息不估造。 |
| 稳定身份 | 原合并键不能完整表达两条流及音频语言 | 单流旧 hash 算法完全保持；合并 `streams-v2` 使用两流独立稳定键，音频键包含语言/声道/采样率。CDN 镜像后缀、签名 URL、大小不作为身份。重提取匹配失败返回 FORMAT_UNAVAILABLE，不换音轨/编码/画质。 |
| 进度 | filename 累加不可靠，估算量和 ETA 缺失，桥接漏 ETA | 按实际 format_id 维护每流最高已下载计数；重复 finished 与重试不会再次累加。两流预知大小可聚合，未知仍未知；进度/速度/ETA 验证有限非负及范围。可选 `total_bytes_estimate`、`eta_seconds` 贯通 Python→Rust→UI，估算显示“约”，合并阶段显示合并。 |
| 完整性 | 传输字节、估算和合并文件大小用途不同 | exact total 才参加终值校验；estimate 不作为完整性证据。YouTube `transferred_bytes` 对照传输计数，`bytes` 为最终合并文件大小；无进度、字节不一致或引擎异常不得完成。Rust 仍校验文件和提交数据库。 |
| 错误 | 任意 401/403 等数字子串可能误分类，原因链丢失 | 优先遍历 DownloadError.exc_info、cause、异常链，识别 HTTP/超时/断流/不完整/合并失败；仅无法取得结构信息时匹配带 HTTP 标签的有限文本。401/403→ACCESS_DENIED，404→NOT_FOUND，429→RISK_CONTROL，5xx/连接/DNS→NETWORK_ERROR，超时→TIMEOUT，不完整→INCOMPLETE_MEDIA，缺规格→FORMAT_UNAVAILABLE。只返回稳定码，保留目标拒绝的 network_target stage。 |

## 参数与重试

| 参数 | 原值 | 新值/边界 |
| --- | --- | --- |
| `retries` | 0 | 视频 HTTP 为 1 次原生重试，固定等待 1 秒；仅 HttpFD 原生判定的 5xx、TransportError、传输截断等路径。401/403/404/429、身份/规格错误不重试。图集保留 0，避免影响它现有的计数契约。 |
| `extractor_retries` / `fragment_retries` / `file_access_retries` | 0 | 保持 0。没有应用自建下载循环。 |
| `socket_timeout` | 30 秒 | 保持 30 秒；两次连续 socket 超时加一次等待为约 61 秒，在 Rust 65 秒无消息等待内；合并仍单独限时 600 秒。原生取消/EOF/Job 或进程组回收保持。 |
| `continuedl` | False | 保持 False。原生在同一次下载内处理瞬态网络错误不等于跨任务续传。 |
| `http_chunk_size` | 提取器/原生默认 | 默认不强制加速，保留原提取器选项。YouTube 可在设置中选择继承、关闭或 1–1024 整数 MiB 自定义；旧探针仍支持环境变量 `YINGQU_HTTP_CHUNK_SIZE=0` 或 `10485760`。显式模式覆盖已选流的 extractor chunk 设置，避免 Python API 的 `0 or downloader_options` 回退。 |

固定版 YouTube 本来会为部分直链注入 10 MiB 分块。因此“继承上游默认”不能笼统称作无分块。YouTube 配置通过既有任务快照传给 Python，优先于启动时的探针环境变量（包括显式继承）；未携带新字段的旧探针仍兼容原环境变量。仅作用于已选媒体的 yt-dlp HTTP 下载器。代理、Cookie、请求头、Referer、GuardedRH 和重定向边界保持。

Range 回归揭示固定版 HttpFD 在提前 EOF 时可能先走不计重试次数的 NextFragment。既有网络适配层现校验 206 的 Content-Range/响应长度，并在已知 Content-Length 提前结束时发出 IncompleteRead，让原生有上限重试生效；不新增下载器或独立请求路径。

v1.7.2 发布前独立审查补充修复：合法 206 响应可使用 chunked 传输且省略 Content-Length；存在该字段时按数值核对，允许前导零，缺失时从已验证的 Content-Range 推导本段预期长度。以本地服务经真实 HttpFD/GuardedRH 验证继承、关闭、10 MiB 三模式下的 chunked/前导零长度完整下载，以及 chunked 提前 EOF 的两次请求上限和禁止报告完成，新增 7 项定向回归 PASS。保持现有目标校验、重试和取消边界。

## 验证

| 命令/场景 | 结果 |
| --- | --- |
| `.venv/Scripts/python.exe -m pytest -q tests/test_video_formats.py tests/test_youtube.py tests/test_engine.py tests/test_http_streaming.py tests/test_probe.py` | PASS，229 项；最后语言覆盖修正另由全量测试验证。 |
| `.venv/Scripts/python.exe -m pytest -q` | PASS，341 项，89.80 秒。 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1` | PASS，100 passed，2 个既有在线用例 ignored。 |
| `cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe` | PASS，随后运行真实 Python/Rust JSONL 集成。 |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASS。 |
| `npm run build` | PASS，保留已有构建提示。 |
| `.venv/Scripts/python.exe -m pytest -q -s tests/test_http_streaming.py` | PASS，36 项：提取器默认/显式关闭/10 MiB，200、206、忽略 Range、异常 Content-Range、未知长度、断流、401/403/404/429/503、瞬态恢复、重试次数和字节一致性。 |
| `tests/test_probe.py` | PASS，包含两种分块模式的下载取消/EOF、进程回收、暂存清理、无进度完成拒绝、ETA/估算字段转发与非法值拒绝。 |
| `tests/ytdlp-variants.browser.js` + `tests/fixtures/background-download.html` | PASS，规格切换/同规格/旧记录/缺文件/竞态、ETA、估算、合并，Space 键及 1440×900、1280×800、1024×680。 |
| 既有 `tests/background-download.browser.js` | PASS，15 个场景；明确隔离的 IPC fixture，不等于 Tauri 原生运行。 |
| PyInstaller + `scripts/check_packaged_engine.py` | PASS，最终源码重建，隔离 cwd/PATH 握手 0.7.4/v3；同一产物的在线解析返回 RISK_CONTROL/network_target，按真实失败记录。 |
| `git diff --check` 与本轮未跟踪源码检查 | PASS，涵盖本轮跟踪/未跟踪文本，不包含用户 `output/`。 |

使用项目现有 Playwright CLI 与 Edge，先 `npm run dev`，再 `playwright-cli -s=ytdlp open http://127.0.0.1:1420/tests/fixtures/background-download.html --browser=msedge`，将对应 browser.js 全文传给 `run-code`。截图和脱敏运行证据位于 `.local/ytdlp-progress-*.png`、`.local/ytdlp-*.log`、`.local/ytdlp-special-20261021/`。

打包命令使用隔离目录，未覆盖用户已有便携包：

```powershell
.venv/Scripts/python.exe -m PyInstaller --noconfirm --distpath .local/ytdlp-package/dist --workpath .local/ytdlp-package/build python-engine/pyinstaller.spec
.venv/Scripts/python.exe scripts/check_packaged_engine.py .local/ytdlp-package/dist/video-engine/video-engine.exe
```

### 速度与真实下载

本机受控代理服务、10,498,117 字节范围响应：显式关闭分块单次 0.0261 秒/1 请求，10 MiB 单次 0.1627 秒/2 请求，两者字节完全一致。忽略 Range 的 200 也通过。此结果只证明正确性与开关真实生效，不代表 CDN 性能收益。

YouTube 样本为用户提供的 `https://www.youtube.com/watch?v=g6dyF2dkVfg`。未读取私人浏览器 Cookie，源码在线操作禁用默认浏览器提取；打包解析只使用虚构 `probe=anonymous` 防止读取浏览器。实测如下。

第一次继承上游 chunk 的运行仅作为格式/FFmpeg 探查，**不计入关闭分块 A/B**。正式对比固定 1080P AV1 + AAC 规格 ID `b72828afbc729835d0eaca22`；记录传输字节、最终文件字节、总耗时、平均速度、FFprobe 轨道和 SHA-256。各模式单次、顺序执行，包含重新提取与合并耗时，不能据此断言长期速度或失败率，也不足以启用强制默认加速。

| 真实模式 | 结果 | 传输/最终字节 | 总耗时 | 平均速度 |
| --- | --- | --- | --- | --- |
| 默认继承 YouTube 10 MiB，较早探查运行 | PASS，仅观察基线 | 88,424,284 / 88,693,532 | 216.61 秒 | 0.389 MiB/s |
| 显式关闭分块 `0` | PASS | 88,424,284 / 88,693,532 | 721.54 秒 | 0.117 MiB/s |
| 显式 `10485760`，正式顺序对比的第二次提取 | UNAVAILABLE；提取失败 RISK_CONTROL/network_target，未进入媒体下载 | 无可用媒体数据 | 提取失败耗时 2.05 秒 | 无可用值 |

两次完成下载均为视频 `399` / AV1 1920×1080 与音频 `140-drc` / AAC，FFprobe 确认两轨，输出 SHA-256 均为 `0fc8ff3420843e356c60584b1573569d69affcc85f60f785b9f0b8df7b423aab`。传输大小与合并输出大小不同，按各自含义记录，未拿估算量校验文件。

正式两模式每种各尝试 1 次：关闭分块 1 次完整成功，强制 10 MiB 在提取阶段 1 次受阻；这不是媒体下载阶段的统计失败率。没有用较早原生 10 MiB 的观察值宣称可靠加速倍率；正式 A/B 受平台验证限制，没有足够性能证据，默认继续继承上游行为。最终产物在线再解析也遇到同类平台验证，未重试或绕过边界。

实际证据：`youtube-ab-validated.json`、`youtube-native-inherited.json`、`youtube-resolve-final.json`、`packaged-resolve.json` 位于 `.local/ytdlp-special-20261021/`。打包文件 SHA-256 为 `9e39635212ab2e50377873b1e230c219ca168835b1306fcbe6a9e167797df211`，来源哈希与独立审查冻结目标一致。

## 兼容与未验证边界

- 协议 v3 只增加可选进度字段；Task 用 serde default 读取旧快照。engine/Python bridge/测试握手同步 0.7.4。现有 cache revision 包含引擎版本，旧解析缓存不会被当作新音轨选择结果使用。
- 单流历史选择 ID 保持；合并 ID 为准确表达音轨而更新，旧合并选择不会静默映射成新音轨。旧合并预览/失败任务可能需要重新解析并选择；否则明确 FORMAT_UNAVAILABLE。已有成功文件/记录保留；没有可靠旧音轨证据时不猜测迁移 ID，因此旧合并 ID 与新 ID 不能证明是同一变体。
- 下载队列、任务 ID/创建时间、目录不覆盖与最终提交机制未改。新增设置仅为 platform_settings 添加可空 http_chunk_size 列，不修改任务表/缓存表。签名 URL 不进入新增公开字段、事件、日志或缓存。
- 本轮仅提供 YouTube 样本；抖音/TikTok 真实 CDN A/B 未执行，未读取已有私人 Cookie。macOS Intel/Apple Silicon、Tauri 原生 GUI、全新无 Python Windows 安装验收 NOT_RUN；Windows onedir/浏览器 IPC 结果不能替代这些验收。
- 自动审批拒绝中止探查后临时目录的递归清理，只返回 `blocked by policy`；`.local/ytdlp-special-20261021/youtube-ab-wll8zhhf` 暂存保留，不参与提交。

## 改动清单

- 生产：`python-engine/engine.py`、`video_formats.py`、`network.py`；`src-tauri/src/desktop_core.rs`、`python_bridge.rs`；`src/api.ts`、`src/main.tsx`、新增 `src/lib/download-identity.ts`。
- 回归：`tests/test_engine.py`、`test_video_formats.py`、`test_youtube.py`、`test_probe.py`、新增 `test_http_streaming.py`；`tests/conftest.py`；`tests/fixtures/desktop_engine.py`、`background-download.html`；新增 `tests/ytdlp-variants.browser.js`。
- 文档：本文件、`docs/youtube-validation.md`、`docs/tauri-desktop-technical-plan.md`、`README.md`。

## YouTube 分块设置补充（2026-10-21，引擎 0.7.5）

按用户后续要求增加系统设置入口，默认选择“继承上游默认”，也可“关闭分块”或“自定义”1–1024 整数 MiB，初始自定义值 10 MiB。保存复用现有平台配置命令，Cookie/代理/UA 与分块值在同一条 SQL 中保存；读取 Cookie 失败时仍可保留 Cookie 保存分块值。配置与恢复规则见 [YouTube 文档](youtube-validation.md#youtube-分块设置2026-10-21)。旧任务/成功记录和文件提交不变，已解析/排队/下载任务保持配置快照，新解析或重试使用当前设置。

| 本次实际命令/检查 | 结果 |
| --- | --- |
| `.venv/Scripts/python.exe -m pytest -q` | PASS，最终 366 passed（包含 55 项 HTTP 与 69 项 YouTube 测试）。 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1`（`CI=true`） | PASS，103 passed、2 ignored；忽略项仍为原有在线/系统交互测试。 |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASS。 |
| `npm run build` | PASS；Tauri 编译前也重新执行生产构建，保留已有构建告警。 |
| Playwright CLI + Edge，`tests/youtube-chunk-settings.browser.js` | PASS，6 场景组，覆盖三视口及三模式/键盘/校验/保存/恢复/失败/隔离。 |
| Playwright CLI + Edge，`tests/platform-settings.browser.js` | PASS，已有 10 场景组。 |
| `.venv/Scripts/python.exe -m PyInstaller --noconfirm --distpath .local/youtube-chunk-settings-20261021/engine-dist --workpath .local/youtube-chunk-settings-20261021/engine-build python-engine/pyinstaller.spec` | PASS，重新收集并打包最终 Python 源码与依赖。 |
| `.venv/Scripts/python.exe scripts/check_packaged_engine.py .local/youtube-chunk-settings-20261021/engine-dist/video-engine/video-engine.exe` | PASS，隔离 cwd/PATH 握手 0.7.5 / yt-dlp 2026.08.19 / v3；实际打包引擎另验证 3 个非法分块配置均返回 CONFIG_INVALID。 |
| `npm run tauri -- build --features desktop --config D:/wx_lifeilin/github.com/lifei6671/video-downloader/.local/youtube-chunk-settings-20261021/tauri.preview.conf.json --no-bundle`（预览版本由 `APP_VERSION` 注入） | PASS，Windows x64 GUI 可执行文件，完整 sidecar 资源复制逐文件哈希一致，前端包含新设置和预览版本。未修改仓库版本或提交/发布。 |

新增 HTTP 用例由本地受控服务验证：继承上游 2 MiB、显式关闭、1 MiB 和 3 MiB 均经真实 yt-dlp HttpFD/GuardedRH 下载相同规格，完整字节一致；分离音视频测试验证两个流都采用当前配置且不改变音轨/规格 ID。没有新增真实 CDN 性能数据，前述在线 A/B 的限制仍适用。

预览目录：`output/Yingqu-1.7.1-preview.20261021-youtube-chunks-win64/`，版本 `1.7.1-preview.20261021.youtube-chunks`；`yingqu.exe` SHA-256 `dfbe79382163d8ad088b1cc0d5ad5af88a131a18638eb34e2f7244cc0b73da9c`，引擎 SHA-256 `617b6628637fb3a9f0124fcf5fac41ac34175b81ac38a41bac0a8783fc7e9b90`。校验清单及来源信息随目录保存。原生 GUI 保存/实际平台下载由用户预览验收，本次未自动退出正在运行的旧预览；macOS 与全新 Windows 安装仍 NOT_RUN。日志与截图在 `.local/youtube-chunk-settings-20261021/`。
