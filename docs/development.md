# 开发、验证与发布

本文面向开发者，以下命令均在仓库根目录执行。产品介绍与使用流程见[项目首页](../README.md)。

## Windows 开发与测试

需要 Rust/Cargo、uv；在仓库根目录 PowerShell 执行：

```powershell
uv venv --python 3.12 .venv
uv pip sync --python .venv/Scripts/python.exe python-engine/requirements-dev.txt
cargo build --manifest-path src-tauri/Cargo.toml --bin m1-probe
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
.venv/Scripts/python.exe -m pytest -q
```

pytest 包含 Rust/Python 进程集成测试，必须先构建探针；测试使用隔离 HTTP fixture，不访问真实平台。运行依赖固定 yt-dlp 2026.8.19 与 DTK 5.1.3 白名单切片，协议 v3，具体版本见锁文件。

### Windows P1A 隔离原生验收

使用已配置的开发环境和合成视频，不使用日常资料库。先在一个终端运行 `npm run dev`，在另一个 PowerShell 终端执行：

```powershell
# Python 源码改变后，先更新现有 sidecar。
.venv/Scripts/python.exe -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec
.venv/Scripts/python.exe scripts/check_packaged_engine.py python-engine/dist/video-engine/video-engine.exe
pwsh -NoProfile -File scripts/p1a-acceptance.ps1 -Scenario empty -RunId p1a-check
```

`Scenario` 可选 `empty`、`upgrade`、`restore`。`upgrade` 生成重复作品、两个文件版本、缺失视频、旧原文和图集混合历史；`restore` 提供空知识库。运行器预建 `%APPDATA%/me.disign.yingqu.p1a-<RunId>-<Scenario>/`，媒体、独立 EXE 与日志放在 `.local/p1a-acceptance/<RunId>/<Scenario>/`。已存在但没有匹配标记的目录会被拒绝；相同参数重跑保留数据库和媒体指纹，重建前须退出该场景的 EXE。`-BuildOnly` 只准备资料并编译。

验收 EXE 复用生产界面、命令和 Core，使用编译时独立标识，并在生产 setup 之前检查实际资料路径的标记。不要修改生产 `identifier` 来隔离测试。Windows Cargo example 需要与正式 bin 一样的 `resource.lib`，包含 Common Controls v6 manifest 和图标；仅复制对话框代码或只验证编译不能证明可启动。

该入口用于 Windows 源码开发验收，需要 Vite 和本机已打包的引擎；不是安装包或 macOS 验收入口。记录与阶段边界见 [P1A 集成验收](workbench-knowledge-base-validation.md#2026-10-27p1a-集成与-windows-原生验收)。

### WB-201 MCP SDK 验证

使用已安装的 Rust、Node 和现有 `node_modules`，在仓库根目录执行。仅构建独立合成 probe，不启动生产应用：

```powershell
cargo build --locked --manifest-path tests/mcp-sdk-probe/Cargo.toml
cargo clippy --locked --manifest-path tests/mcp-sdk-probe/Cargo.toml --all-targets -- -D warnings
node scripts/verify-mcp-sdk.mjs
```

上述脚本复用 `package-lock.json` 中 shadcn 已依赖的官方 TypeScript SDK；不带 `--codex` 时真实 AI 客户端项明确为 `NOT_RUN`。首次构建下载固定的 Cargo 依赖，需要网络；后续可按缓存离线构建。

验证已安装的 npm 版 Windows x64 Codex CLI（没有安装时不要用本命令推断已测）：

```powershell
$codexLauncher = (Get-Command codex -ErrorAction Stop).Source
$codexExe = Join-Path (Split-Path $codexLauncher) 'node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe'
if (!(Test-Path -LiteralPath $codexExe)) { throw '请指定已安装 Codex 的原生可执行文件绝对路径' }
node scripts/verify-mcp-sdk.mjs --codex $codexExe
```

脚本在 `.local/wb201/run-*/` 创建独立配置、含中文和空格的 EXE 路径及报文证据。它只调用 Codex app-server 的工具发现，不创建对话、不开始模型 turn、不使用登录数据，不修改日常 Codex 配置。可执行文件路径通过参数数组传递，不拼 shell 命令。

当前 Windows 编译与连接证据、SDK 版本 / 许可、协议差异和 macOS 待办见 [MCP 选型验证](mcp-sdk-validation.md)。验证样例不加入正式安装包，生产接入另由 WB-202～WB-208 实施。

### WB-302 段落存储与媒体时钟验证

源码、协议、schema 和备份格式一起变更后，先构建精确版本的 Rust 探针，再执行 Python 进程集成测试；旧探针会因 engine 0.16.0 握手不符而失败。

```powershell
cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe --example timing_acceptance
.venv/Scripts/python.exe -m pytest -q
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
cargo doc --locked --manifest-path src-tauri/Cargo.toml --features mcp --no-deps
npm run build
.venv/Scripts/python.exe -X utf8 scripts/verify_transcript_clock.py --output .local/wb302/clock-local
```

时钟矩阵用现有 FFmpeg / 同目录 FFprobe 生成低分辨率测试素材，生产提取函数直接参与断言；FFprobe 仅用于验证。保留 PTS 零点，不能加入 `-start_at_zero` 减去首包时间。脚本校验 11 组音轨延迟、容器起点、负 PTS、VFR、PCM / AAC / Opus，codec frame 容忍不代表词级或播放器同步精度。

真实 Core 验收使用 `timing_acceptance` 例子：先在 `.local` 下新建资料目录，写入 `fixture.json` 为 `{"purpose":"wb302-synthetic"}`，按既有模型清单在其 `runtime/fun-asr-nano` 或 `runtime/fun-asr-nano-cuda` 放入已验证的完整运行目录，不使用日常资料库。配置示例：

```json
{
  "root": "<仓库 .local 下的新资料目录绝对路径>",
  "program": "<本机 .venv/Scripts/python.exe 的绝对路径>",
  "args": ["<仓库 python-engine/bridge.py 的绝对路径>"],
  "video": "<本轮 clock-local/delay.mkv 的绝对路径>",
  "backend": "cpu"
}
```

```powershell
src-tauri/target/debug/examples/timing_acceptance.exe '<配置 JSON 路径>'
.venv/Scripts/python.exe -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec
.venv/Scripts/python.exe scripts/check_packaged_engine.py python-engine/dist/video-engine/video-engine.exe
```

运行器拒绝已有数据库和未标记目录，真实导入 / 转录两次，检查段落、旧版本、重开和源文件不变，再关闭 Core。CUDA 用新的独立资料目录及 `backend=cuda`；打包验收使用另一个新目录，`program` 指向新构建的 `video-engine.exe`、`args=[]`。既有模型文件可在同卷测试目录硬链接，安装 manifest 不变，不触发下载或修改后端日常选择。保存每轮 `report.json`，不能把首轮旧构建记录代替最终产物。

浏览器播放坐标需另以支持 byte ranges 的本地服务器和已安装 Edge 检查，不能使用不支持 Range 的简单 HTTP 服务器冒充 seek 测试。这里的 native Core / sidecar 与完整 Tauri UI、安装包、实际音画同步分别记录；macOS 两种架构仍需独立复验。字段、备份兼容和限制见 [段落契约](transcript-segments.md)。

### WB-301 文案时间来源验证

仅使用现有 CPU / CUDA 运行目录，不安装新模型。输入为 16kHz、单声道 PCM16 WAV，输出必须是仓库 `.local` 下的新目录；存在时拒绝覆盖。自然长口播使用已有隔离测试音频，不能指向日常资料库或改写原视频。

```powershell
.venv/Scripts/python.exe -m pytest -q tests/test_transcript_timing.py tests/test_transcription.py
.venv/Scripts/python.exe -X utf8 scripts/verify_transcript_timing.py --cpu-directory '<现有 Nano CPU 目录>' --cuda-directory '<现有 Nano CUDA 目录>' --english-audio '<官方英文 PCM16 WAV>' --natural-audio '<隔离自然长口播 PCM16 WAV>' --output .local/wb301/run-local
.venv/Scripts/python.exe -X utf8 scripts/verify_transcript_clock.py --output .local/wb301/clock-local
```

没有已验证 CUDA 环境时省略 `--cuda-directory`，相应项明确为 `NOT_RUN`；macOS 只复验其 CPU 路径，不要求 Windows CUDA。主实验调用真实 VAD、CPU `--srt` 和生产 CUDA 编排，可能持续数分钟，日志与 JSON 保留在新目录。核对裁剪实际采样、片段对应和源文件哈希；不把结构检查当作完整文本准确率或词边界精度。

macOS 沿用同一运行器，使用已有模型目录与独立测试输入，例如：

```sh
.venv/bin/python scripts/verify_transcript_timing.py --cpu-directory '<已有 Nano CPU 目录>' --english-audio '<官方英文 PCM16 WAV>' --natural-audio '<隔离完整长语音 PCM16 WAV>' --output .local/wb301/macos-new-run
```

2026-10-29 的完整 628.541 秒解说和 466.782 秒具名朗读结果见 [Apple Silicon 补验](transcript-timing-validation.md#macos-apple-silicon-长语音补验2026-10-29)。前者跑完整四组主实验并经 Tauri 真实转录入库；后者另以既有 VAD / SRT 校验函数和生产 `transcription.recognize` 交叉核对，没有原生入库。新样本只用于本地验证，保留来源、完整输入哈希和转换记录，不加入安装包。

官方英文输入来自固定的 `https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-2512/resolve/272c57b82523ada6fd87095e955f8e29100979ab/example/en.mp3`，本轮原文件 SHA-256 为 `f10378336a4e584f3f63799e62f99d5add3c2a401b51d3abe7d3a3a82f255ada`。如本地没有该样本，可下载到新的测试输入目录，用已安装 FFmpeg 转为所需格式，再记录转换结果哈希：

```powershell
& '<已安装的 ffmpeg 绝对路径>' -nostdin -v error -n -i '<测试目录>/en.mp3' -ar 16000 -ac 1 -c:a pcm_s16le '<测试目录>/en.wav'
```

媒体时钟检查另需现有 FFmpeg 同目录的 FFprobe，仅用于确认 0 / +1500ms 音轨起点，不新增产品依赖。该探针生成低分辨率黑色视频与原有官方短语音，比较当前提取和候选滤镜后的实际 PCM。当前滤镜、原生播放器和其他容器的完整接入尚未实现；复验范围、原始结果及 WB-302 前置条件见 [时间来源与精度验证](transcript-timing-validation.md)。

### WB-208 真实客户端与模型验收

先完成下节的独立 NSIS 安装，使用唯一 identifier / productName / mainBinaryName，并在第一次启动前为实际 Roaming 资料目录创建匹配 `wb208-acceptance.json` 标记：`purpose` 为 `wb208-synthetic`，`workspace` 为仓库绝对路径。应用空库初始化后退出，才运行构造器；不要指向日常库。

```powershell
node --test tests/mcp-client-checks.test.mjs
.venv/Scripts/python.exe -m pytest -q tests/test_mcp_acceptance.py
.venv/Scripts/python.exe -X utf8 tests/fixtures/seed_mcp_acceptance.py --data '<本轮标记的实际资料目录>' --output .local/wb208
```

从安装实例启用 MCP，捕获复制的 URL；捕获文件须与 `tests/mcp-installed.browser.js` 输出形状一致，包含 `status=PASSED`、`json` 和实际 `info`。该既有脚本的截图固定写 `.local/wb207/installed-settings.png`，跨批复用时先保全旧证据，使用本批独立文件名归档，不覆盖历史证据。正常原生启动也可通过设置 UI 操作，无须为重启增加调试参数。

仅在用户明确允许使用本机登录与合成资料进行模型调用后，加 `--allow-account`。在持续保留 stdin 的交互终端 / PTY 中执行，使用前文定位到的原生 Codex EXE：

```powershell
node scripts/verify-mcp-client.mjs $codexExe .local/wb208/installed-copy.json .local/wb208/expected.json --allow-account
```

不带该开关时使用隔离 home，仅执行协议与生命周期，模型为 `NOT_RUN`。授权模式由 Codex 自己使用登录存储，脚本不复制凭据、不改写日常配置；逐会话禁用继承 MCP，要求只有 yingqu 提供工具。实际模型开始前会输出 `model_running`；此后不可把非交互运行当作完整验收。

运行器输出以下 phase 后，在另一个终端 / 原生界面完成对应操作，再向运行终端输入一行 JSON，例如 `{"state":"service_stopped"}`：

| phase | 完成操作后再输入同名 state |
| --- | --- |
| `service_stopped` | 从本轮应用设置关闭 MCP |
| `service_started` | 重新开启 MCP |
| `application_closed` | 仅终止已核实路径的本轮隔离主程序，模拟故障退出 |
| `application_restarted_off` | 普通重启本轮程序，确认默认关闭 |
| `application_restarted_on` | 从设置开启 MCP |

最终 `.local/wb208/client-*/summary.json` 需要同时通过工具、模型（授权模式）、生命周期与自然退出。模型引用结构的自动审计不替代内容质量审阅。协议 / 模型之前和之后用构造器的 `--snapshot` 对比逻辑表，期间不要在 UI 访问资产；来源导航会明确记录访问。随后在原生界面核对报告的资产、文档版本、笔记修订和引用原文。分别记录清理、安装包、Windows 与两种 macOS 结果，详见 [WB-208 验收](mcp-client-acceptance.md)。

### WB-207 MCP 构建与安装包验证

已有引擎按本文命令准备后，使用完整 feature 构建以填充 Cargo cache，再核对许可和安装内容：

```powershell
.venv/Scripts/python.exe -m pytest -q tests/test_mcp_packaging.py
npm run tauri -- build --features desktop,mcp --config src-tauri/tauri.release.conf.json --bundles nsis -- --locked
.venv/Scripts/python.exe -X utf8 scripts/update_mcp_licenses.py --check
$installer = Get-ChildItem src-tauri/target/release/bundle/nsis/*-setup.exe | Select-Object -First 1
pwsh -NoProfile -File scripts/check_windows_installer.ps1 -Installer $installer.FullName -RunId local-check
```

检查器实际安装到新的 `.local/wb207-install/<RunId>/安装 Program Files` 并校验、卸载；拒绝覆盖已有目录。生产安装器会注册正常产品项，交互验收须按 [分发文档](mcp-packaging.md)使用独立 identifier、productName 与 mainBinaryName 的本地覆盖配置，并预建独立数据目录，避免接触日常配置和旧资料迁移；不要在当前日常应用运行时执行生产安装验收。

`-KeepInstalled` 仅用于成功后保留原生验收实例；改名的验收包同时传 `-MainBinary yingqu-wb207.exe`。使用 WebView2 独立调试端口连接安装实例后，执行 `tests/mcp-installed.browser.js` 捕获配置，再运行：

```powershell
node scripts/verify-mcp-config.mjs .local/wb207/installed-copy.json
# 从界面关闭服务后：
node scripts/verify-mcp-config.mjs .local/wb207/installed-copy.json --unavailable
```

JSON 捕获方式与 WB-206 原生检查一致；该浏览器脚本要求安装后的内嵌前端，不能对 Vite fixture 运行。地址验证器只接受捕获文件和可选的 `--unavailable`；真实 Codex 验证另按 WB-208 的 `verify-mcp-client.mjs` 执行，不能以 SDK 连接成功推断模型已消费内容。原生验收涉及真实剪贴板，验证完应退出并卸载本轮独立实例。macOS 分别执行原生构建、`scripts/check_packaged_mcp.py '<映取.app 路径>'` 和签名 / 客户端验证，当前仍待设备。

### WB-206 MCP 设置与配置验证

```powershell
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib mcp_connection
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
npm run dev
```

另一个终端用已有 Playwright CLI 执行合成界面验证：

```powershell
playwright-cli -s=wb206 open 'http://127.0.0.1:1420/tests/fixtures/knowledge-library.html?mcp=1' --browser msedge
playwright-cli -s=wb206 --raw run-code --filename=tests/mcp-settings.browser.js
```

原生验证复用 `scripts/p1a-acceptance.ps1` 的独立标识与带 marker 的空库，使用唯一 RunId 构建；只需启动输出的隔离桌面程序；HTTP 服务内置于 Core，不复制额外适配程序。`mcp-ipc-fixture` 是 SDK 合成测试入口，不是桌面运行依赖。为该隔离程序设置 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9236` 后启动，恢复环境变量，再通过 `playwright-cli -s=wb206-native attach --cdp http://127.0.0.1:9236` 连接。不要对日常应用运行此项；测试需要真正写入剪贴板，且起始服务应为关闭。

```powershell
playwright-cli -s=wb206-native --raw run-code --filename=tests/mcp-settings-native.browser.js > .local/wb206/native-copy.json
node scripts/verify-mcp-config.mjs .local/wb206/native-copy.json
# 从设置页关闭服务后，旧配置必须连接失败：
node scripts/verify-mcp-config.mjs .local/wb206/native-copy.json --unavailable
```

地址验证器使用已安装的官方 TypeScript SDK，直接连接捕获的本机 HTTP 地址，检查工具发现与分类读取；它不启动 Codex、不发起模型请求。捕获文件包含 URL 和服务状态，作为本地证据保存，不提交。命令假设 `.local/wb206` 已建立，正式安装包与 macOS 分开验收，详见 [设置契约](mcp-settings.md)。

### WB-202 本机 IPC 验证

仅使用临时资料库和合成分类 / 标签，不访问日常应用数据：

```powershell
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib core_ipc -- --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml --test core_ipc -- --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings
```

第一条覆盖本机传输、请求边界及 Core 生命周期；第二条启动独立测试进程验证原生通信与退出断连。输出中的 `ipc_child_fixture` 被标为 ignored，是由父测试以 `--ignored --exact` 实际执行的子进程入口，不是遗漏的验收项。Windows 额外核对实际命名管道的受保护 DACL；同一组命令在对应 Mac 原生环境运行才能验收 Unix socket，当前两种 Mac 结果均保留 `UNAVAILABLE`。

桌面后端已注册状态和启停命令，IPC 默认关闭且重启关闭；本批没有添加设置 UI 或生产 MCP 适配进程。内部 JSON Lines 不是 MCP stdio，不能将该管道直接配置为 AI 客户端的 MCP Server。详细契约见 [应用核心本机 IPC](mcp-local-ipc.md)。

<a id="wb-203-stdio-适配进程验证"></a>

### WB-203 MCP 协议与生命周期验证

使用已安装的 Rust / Node / node_modules，构建可选 `mcp` feature 和合成 Core fixture：

```powershell
cargo build --locked --manifest-path src-tauri/Cargo.toml --features mcp --example mcp-ipc-fixture
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib mcp_adapter -- --test-threads=1
node scripts/verify-mcp-adapter.mjs
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
```

测试运行器复用已安装的官方 TypeScript SDK；不新增 npm 依赖。它创建 `.local/wb203/run-*/`，启动隔离 Core 的 HTTP 服务，以合成分类和标签验证旧 initialize、现代 discovery、调用、Core 服务关闭、连接关闭、应用退出及自然进程回收，保存 summary、双向报文及 stderr。Core fixture 需要专用环境标记和新资料库目录，不访问日常数据；它是开发样本，未加入安装包。

HTTP 服务随 `desktop` feature 内置，不再构建或分发命令适配程序。工具与错误契约见 [Streamable HTTP 接入](mcp-streamable-http.md)，正式分发检查见 WB-207。Mac 应在各自原生环境执行同组命令；当前 Intel / Apple Silicon 均未执行，不将 Windows 结果替代。

### WB-204 七项只读工具验证

在 WB-203 构建入口上复验：

```powershell
cargo build --locked --manifest-path src-tauri/Cargo.toml --features mcp --example mcp-ipc-fixture
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib mcp_read -- --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib mcp_read -- --test-threads=1
node scripts/verify-mcp-adapter.mjs --library
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
```

`--library` 使用 `.local/wb204/run-*/`，在现有隔离 Core fixture 中启用多资产、多文档版本、人工 / AI 笔记及引用样本；`scripts/mcp-read-checks.mjs` 验证七工具的真实 SDK / Streamable HTTP / 内部 IPC 结果和错误，并继续执行 WB-203 的握手及关闭回收检查。不传该参数仍验证原有合成生命周期，工具发现按当前完整工具清单断言。

公共 DTO 只对可选 mcp feature 派生 SDK Schema；不启用 SDK 的 Core 定向测试仍可运行。未新增依赖、修改库结构、UI、配置或分发。具体输入、游标、版本和读取范围见 [只读工具契约](mcp-read-tools.md)。Mac 两种架构继续需要对应原生环境执行，本轮 Windows 结果不替代。

### WB-205 Resources 与边界验证

```powershell
cargo build --locked --manifest-path src-tauri/Cargo.toml --features mcp --example mcp-ipc-fixture
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib mcp_adapter::resources -- --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib mcp_read -- --test-threads=1
node scripts/verify-mcp-adapter.mjs --resources
node scripts/verify-mcp-adapter.mjs --library
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
```

`--resources` 将证据写入 `.local/wb205/run-*/`，启用共享 fixture 中的合成凭据和转义 ID 样本，并由 `scripts/mcp-resource-checks.mjs` 深比较资源 / 工具结果及失败范围。凭据仅为合成哨兵，不读取系统 credential store 或日常资料库。不带资源模式的 `--library` 仍完成七工具读取，验证 Tools 不依赖 Resources。

URI 解析复用现有 url 和已锁定的 percent-encoding 2.3.2；后者只增加可选 mcp feature 的直接依赖边，锁文件没有新增包或版本变更。本批没有增加库结构、UI 或分发配置。URI / 错误和平台边界见 [Resources 契约](mcp-resources.md)；macOS 两种架构仍需对应机器原生验收。

## macOS 开发与打包

需要 Xcode Command Line Tools、Rust/Cargo、Node/npm 和 uv；Intel 与 Apple Silicon 必须分别在对应架构的 Mac 上构建 Python 引擎，不混用 Windows 产物或另一架构的 `.venv`。以下命令在仓库根目录的 zsh 执行：

```sh
uv venv --python 3.12 .venv
uv pip sync --python .venv/bin/python python-engine/requirements-dev.txt
uv pip check --python .venv/bin/python
npm ci
cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe
cargo test --locked --manifest-path src-tauri/Cargo.toml
.venv/bin/python -m pytest -q
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings
.venv/bin/python scripts/build_asr.py
.venv/bin/python -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec
.venv/bin/python scripts/check_packaged_engine.py python-engine/dist/video-engine/video-engine
npm run desktop
```

生成包含完整引擎的应用与磁盘映像：

```sh
APPLE_SIGNING_IDENTITY=- npm run tauri -- build --features desktop,mcp --config src-tauri/tauri.release.conf.json --bundles app,dmg -- --locked
codesign --verify --deep --strict src-tauri/target/release/bundle/macos/映取.app
.venv/bin/python scripts/check_packaged_engine.py src-tauri/target/release/bundle/macos/映取.app/Contents/Resources/video-engine/video-engine
.venv/bin/python scripts/check_packaged_mcp.py src-tauri/target/release/bundle/macos/映取.app
open src-tauri/target/release/bundle/macos/映取.app
```

应用位于 `src-tauri/target/release/bundle/macos/映取.app`，DMG 位于相邻的 `bundle/dmg/`。`-` 是本地 ad-hoc 签名；按 2026-10-14 用户决定，macOS 不要求 Developer ID、证书或公证，不将这些设置为本次验收阻塞项。本地签名检查不代表下载后的 Gatekeeper 场景已验证。

当前 Tauri CLI 按 Cargo 的 `[[bin]]` 声明顺序签名，因此 `m1-probe` 必须排在主程序 `yingqu` 前，避免主程序签名检查同包中尚未签名的辅助程序。日常 CI 的两种 macOS 架构均配置生成 ad-hoc 签名的 `.app`，并执行 `codesign --verify --deep --strict`；Release 另生成 DMG。本轮未运行远程 CI 或 Mac 硬件验收。

`tauri.macos.conf.json` 自动合并，声明最低 macOS 13.3，以匹配现有 Tailwind 4 的 Safari/WebKit 16.4 要求；13.3 实机兼容性仍待验证。依据：[Tailwind 浏览器要求](https://tailwindcss.com/docs/compatibility)、[WebKit 16.4](https://webkit.org/blog/13966/webkit-features-in-safari-16-4/)。Apple Silicon 本机证据及剩余验收见 [macOS 验证记录](macos-validation.md)。

开发依赖锁文件包含平台标记：`macholib` 只在 macOS 安装，`pefile` / `pywin32-ctypes` 只在 Windows 安装。更新锁文件使用 `uv pip compile --universal python-engine/requirements-dev.in -o python-engine/requirements-dev.txt`，避免再次生成只适用当前主机的清单。

### macOS 工作台与 MCP 隔离验收

复用当前 `.venv`、Node 依赖及已构建引擎；若依赖缺失，按锁文件同步。ASR helper 构建需要 CMake。先在独立终端启动 `npm run dev`，再执行：

```sh
.venv/bin/python scripts/p1a-acceptance-macos.py --scenario upgrade --run-id macos-check
open '.local/p1a-acceptance/macos-check/upgrade/映取隔离验收.app'
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --test core_ipc -- --test-threads=1
cargo build --locked --manifest-path src-tauri/Cargo.toml --features mcp --example mcp-ipc-fixture
node scripts/verify-mcp-adapter.mjs --resources
node scripts/verify-mcp-adapter.mjs --writes
node scripts/verify-mcp-adapter.mjs --operations
```

`--scenario` 支持 `empty`、`upgrade`、`restore`；`--viewport` 支持 `1440x900`（默认）、`1280x800`、`1024x680`。退出同一验收程序后可用相同 run-id 切换窗口尺寸，资料保留。运行器使用编译时独立 identifier，在 `~/Library/Application Support/me.disign.yingqu.p1a-<run-id>-<scenario>/` 创建匹配 marker 后准备合成数据，拒绝未标记或标记不符的目录。生产入口和 example 共用一次 `generate_context!`，避免 macOS 重复嵌入 Info.plist。重跑前退出同一验收程序；重跑保留资料库。

此 `.app` 是依赖 Vite 和仓库引擎的调试入口，不是安装包。协议脚本通过 Streamable HTTP 和内部 Unix socket 连接合成 Core；`--operations` 的识别器是测试夹具，不证明真实 ASR 精度。SDK 选型探针另按 WB-201 命令构建后运行 `node scripts/verify-mcp-sdk.mjs --codex '<本机 Codex 原生程序绝对路径>'`，仅发现工具，不使用登录或模型 turn。安装包、真实模型、原生 UI 及 Intel / Apple Silicon 分开验收，实际结果见[工作台实施记录](workbench-knowledge-base-validation.md)。

## 真实下载验证

TikTok 便捷入口默认读取 `tests/TikTok.txt`、Edge 150 UA 和 SOCKS5 `127.0.0.1:3067`：

```powershell
.venv/Scripts/python.exe scripts/run_m1.py
.venv/Scripts/python.exe scripts/run_m1.py --proxy http://127.0.0.1:3067
.venv/Scripts/python.exe scripts/run_m1.py --direct
```

可用 `--url`、`--cookie-file`、`--user-agent`、`--output-dir` 覆盖 TikTok 样本。双平台或抖音验证使用配置文件。将以下内容保存到忽略目录 `.local/probe.json`，替换有效作品链接；抖音使用直连，TikTok 单独配置代理。UA 应与 Cookie 来源一致，但不代表 TLS 浏览器指纹相同。

```json
{
  "inputs": ["https://www.douyin.com/video/作品ID", "https://www.tiktok.com/@作者/video/作品ID"],
  "profiles": {
    "douyin": {"cookie_file": "tests/douyin.txt", "proxy": null},
    "tiktok": {"cookie_file": "tests/TikTok.txt", "proxy": "http://127.0.0.1:3067"}
  },
  "output_dir": ".local/downloads"
}
```

```powershell
.venv/Scripts/python.exe scripts/run_m1.py .local/probe.json
```

默认 UA 为 Edge 150；可在各 profile 中设置 `user_agent`。`proxy: null` 明确直连，不继承环境代理。没有 Cookie 时省略 `cookie_file`。相对路径按执行时工作目录解释；Cookie 文件是单行 `name=value; name=value`，仅在内存读取，经 stdin 传递，不进入命令参数或日志。`tests/douyin.txt`、`tests/TikTok.txt` 已忽略，其他私有文件放在 `.local/` 或仓库外，不得提交。

探针串行处理输入，每次解析和下载分别启动一个 Python 操作进程。下载重新解析规范链接，核对作品身份后写入任务暂存目录；Rust 验证并提交 `平台_作品ID.mp4/webm`，已有同名文件不覆盖，重复验证使用新输出目录。支持直接 HTTP(S) 单文件视频及图集；图集保存为任务独占目录，按实际格式编号，不转码。直播、分轨、HLS/DASH、转码或断点续传不支持。

`resolved` 表示解析成功，`progress` 表示字节进度，`completed` 表示该文件提交成功。任何输入失败时整体退出码为 1，其他成功文件保留；配置错误为 2。Ctrl+C 取消网络操作、回收进程并清理暂存。独立调用 bridge 时须持续保持 stdin 打开直到进程退出；EOF 会终止操作。M1 不负责异常崩溃后的暂存恢复，桌面任务恢复见 M2 验证记录。

## Windows 引擎打包

```powershell
.venv/Scripts/python.exe -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec
.venv/Scripts/python.exe scripts/run_m1.py .local/probe.json --probe src-tauri/target/debug/m1-probe.exe --engine python-engine/dist/video-engine/video-engine.exe
```

必须一起分发 `video-engine/` 整个目录，包含 `_internal` 与许可证。运行器仅供开发便利；原生探针可直接启动打包 exe，通过 stdin 接收 JSON（调用方将 `cookie_file` 转换为内存 `cookie`）。当前已验证隔离 PATH 与 cwd 的原生调用，尚未在全新无 Python 虚拟机验证，也不是 Tauri 安装包；macOS 需单独构建与实测。

[旧引擎 M1 记录](m1-legacy-validation.md)和[独立 yt-dlp 试验记录](ytdlp-validation.md)仅供历史参考，旧脚本与引擎已退役。

## M2 桌面开发

先按上文构建 PyInstaller 引擎，然后执行：

```powershell
npm ci
npm run desktop
```

前端检查：`npm run build`。完整桌面编译：`npm run desktop:build`（不生成安装器）。Rust 桌面门禁：`cargo test --manifest-path src-tauri/Cargo.toml`、`cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`。

工作台、知识库和下载中心已接入；工作台提供最近访问、待分类、待整理和运行中 / 失败操作入口，访问记录由 Rust 持久化；列表、分类标签、归档、资产详情、播放、文案版本和笔记均通过 `libraryApi` 调用 Rust。文案提供原文只读、整理稿显式保存、修订冲突保留和单篇 TXT；应用内导航接入编辑保护。开发预览运行 `npm run dev`，用已有 Playwright CLI 在 Edge 隔离会话中执行：

```powershell
playwright-cli -s=knowledge-library open http://127.0.0.1:1420/tests/fixtures/knowledge-library.html --browser=msedge
playwright-cli -s=knowledge-library run-code --filename=tests/knowledge-library.browser.js
playwright-cli -s=knowledge-library run-code --filename=tests/knowledge-library-boundaries.browser.js
playwright-cli -s=knowledge-library run-code --filename=tests/library-asset-detail.browser.js
playwright-cli -s=knowledge-library run-code --filename=tests/library-video.browser.js
playwright-cli -s=knowledge-library run-code --filename=tests/library-writing.browser.js
playwright-cli -s=knowledge-library run-code --filename=tests/library-writing-failures.browser.js
playwright-cli -s=knowledge-library run-code --filename=tests/library-backup.browser.js
playwright-cli -s=knowledge-library run-code --filename=tests/workbench.browser.js
```

样本只含合成资产、隔离的 IPC mock 和专用测试存储键，不访问真实视频、数据库、凭据或原生文件选择器。详情脚本使用 `?detail=1`，编辑脚本使用 `?detail=1&writing=1`；工作台脚本使用 `?detail=1&writing=1&workbench=1`，共享可变分类 / 文稿并覆盖空库、失败重试与准确跳转；编辑失败样本覆盖修订冲突、提交后回读失败、未保存导航及转录摘要。测试之间通过界面明确放弃未保存的 fixture 修改，不用全局自动接受浏览器重载来绕过保护。`tests/fixtures/media/` 自生成的 VP8/WebM、H.264/MP4 用于实际浏览器解码；原生打开 / 定位、导出和 ASR 仍是 mock。页面重载验证 fixture 的保存和重新读取，不能代替 Tauri 重启验收。完整证据见 [知识库实施记录](workbench-knowledge-base-validation.md)。

WB-117 新增 Rust Core 导出 / 备份 / 恢复接口，目录包 v1、空库规则与进度取消契约见 [API 第 6 节](workbench-knowledge-base-api.md#6-单篇导出与逻辑包核心wb-117)。`cargo test --locked --manifest-path src-tauri/Cargo.toml library -- --test-threads=1` 覆盖真实临时文件、目录联接、包损坏及事务回滚；`core_backup_restore` 用临时 SQLite 关闭重开校验。WB-118 已接入新格式 / 笔记导出、设置“知识库”分组与六条原生命令，操作通过现有 Core worker 异步执行并支持进度、取消及退出等待。浏览器备份样本使用 `?detail=1&writing=1&backup=1`，原生对话框由 IPC mock 代替；实际包内容与资源回收由临时 SQLite / 文件测试证明，原生 GUI 另验。

在设置页分别保存两平台 Cookie、User-Agent 与 HTTP/SOCKS5 代理，再粘贴作品链接。Cookie 直接存本机应用数据目录的 SQLite，在设置页多行文本框回显；含密码的代理存系统凭据。下载目录默认为系统下载目录下的 Yingqu，可通过系统目录选择器修改。解析最多同时一个，下载默认同时 2 个、可在设置调整为 1–3 个，超出名额按提交顺序等待；提高上限立即补位，降低不打断已有下载。取消等待进程退出与暂存清理；重启保留记录及已解析预览，解析中/排队/下载中标记中断，不自动恢复下载。可复用缓存和封面/图集字节保存在 SQLite，命中时不请求详情，下载时仍重新解析；见 [解析缓存](parse-cache-validation.md)。

并发队列、手动重试、移除历史及删除本地作品已实现。按 2026-10-21 用户要求，单条移除记录和本地删除均直接执行、不二次确认；本地删除同时移除下载任务和数据库下载记录。重试复用原任务记录，保留 ID、创建时间、实际 resolver 与所选规格，清空上次进度和错误，以最新设置重新解析并从头下载；解析失败且无作品信息时需重新粘贴链接。Debug 开发可执行文件依赖仓库内的打包引擎目录；Release 从应用资源目录加载完整引擎。Windows 干净环境与 macOS 验收在 M3 完成。[M2 单任务验证](m2-validation.md) · [并发与队列验证](m2-queue-validation.md) · [重试与历史管理验证](m2-history-validation.md)。

下载作品页从提交解析到当前解析结束期间禁用链接输入、右键编辑、粘贴及重复解析；迟到的剪贴板结果不修改已禁用的输入。预览底部以“后台下载”为主按钮，“立即下载”为次级描边按钮，两者沿用既有后台交接和首页下载流程，不改变 FIFO 队列。下载作品的链接输入框支持右键全选、剪切、复制、粘贴和清空；剪切/复制操作选区，粘贴替换选区或插入光标处，清空移除全部文字。剪贴板失败保留原文，Enter 解析与 Shift+Enter 换行保持可用。浏览器回归使用 `tests/link-input.browser.js` 与隔离的 `tests/fixtures/link-input.html`，不读取真实剪贴板或平台凭据。

下载任务的平台 Tab 行右侧可切换列表和信息流，默认列表，当前会话内保留选择；两种排版共用任务、筛选、搜索、进度和操作。信息流以 3:4 封面卡片组成自适应多列网格，完成徽章在封面左上角、播放在右上角，下载进度在封面中央用圆环和圈内百分比展示，使用 `tests/task-feed.browser.js` 验证切换、状态、键盘与三个设计视口。信息流封面悬停时轻微放大图片；圆环使用半透明底，圆弧与数字按真实目标平滑更新，大小未知或合并时不生成假百分比。封面详情及其他 Dialog/AlertDialog 统一淡入淡出，退出结束再释放遮罩和恢复焦点；减少动态效果时立即切换。下载任务页的标题、搜索框和平台 Tab 栏保持原位，仅 Tab 下方列表滚动；其他页面沿用原来的页面滚动。`tests/task-platforms.browser.js` 通过隔离长列表检查三种窗口尺寸下的滚轮、键盘、Tab 固定位置及筛选行为。

下载任务列表的封面支持右键“保存封面”，通过系统另存为对话框选择位置；无可用封面时禁用，取消不写文件。原样导出缓存或旧版本地封面字节，默认文件名为 `平台_作品ID.cover.实际图片扩展名`，不转码或重新请求平台；图集导出当前显示的第一张图片。保存操作不改变任务、封面缓存或原下载文件。浏览器回归为 `tests/cover-save.browser.js` 与隔离的 `tests/fixtures/cover-save.html`。

同一右键菜单提供“复制封面”：通过浏览器解码已加载图片，保留原始像素尺寸，将 RGBA 图片经现有原生剪贴板插件写入系统剪贴板；不复制 URL 或文件路径，不新增解码依赖。无图时禁用，复制或保存进行时禁用重复操作，失败显示提示。单击封面（或键盘 Enter/Space）通过现有 Dialog 展示大图，复用同一图片 URL，保持比例并限制在窗口内；可通过关闭按钮、遮罩或 Esc 关闭并恢复焦点。图集使用首图。浏览器回归见 `tests/cover-copy-preview.browser.js`；原生 RGBA 参数校验测试在 `desktop.rs`，不读写系统剪贴板。

预览浮层内也可右键图片“另存为…”或“复制封面”，复用相同保存和剪贴板路径；复制时读取当前浮层图片的原尺寸像素。浮层按图片宽高收缩，长标题不撑宽，保留关闭控件所需的最小宽度，并在窗口内等比缩放。菜单可用 Shift+F10 打开，Esc 先关闭菜单，再关闭浮层。尺寸及浮层菜单回归见 `tests/cover-preview-fit.browser.js`。

新下载的抖音、TikTok、YouTube 视频仍获取平台可用的最高分辨率封面，与 Telegram 一样经校验后只存现有 SQLite 封面缓存；下载目录只保存视频，不再生成同名 `.cover.jpg/png/webp`。列表沿用受控封面读取命令并支持重启和提交恢复。封面获取或缓存更新失败不影响视频完成，已有预览封面仍可显示。删除本地视频同时删除对应任务、下载记录和提交证据；移除最后一个同身份任务或预览时，同事务清空封面 BLOB，保留解析元数据、别名及本地文件。旧版封面文件保持原样，不自动迁移、删除或联网补处理；图集原图保存方式不变。行尾的复制原链接、本地删除不再收进二级菜单，和播放、文件夹、列表移除一样使用图标及 Tooltip，本地删除默认红色。`tests/task-actions.browser.js` 覆盖三视口、键盘、无确认、复制、失败、pending 和迟到事件；Rust 历史/封面测试覆盖文件删除、数据库记录清理、失败后重试和重启。批量清理历史保留确认，但只清理当前平台 Tab（全部 Tab 覆盖所有平台），不受搜索条件影响；无元数据的 Telegram 任务按链接归类，后端继续保护进行中的任务与独立解析缓存。单条删除不再聚焦顶部搜索框。成功删除/清理的可见项向上淡出并折叠占位，下方条目平滑上移；失败保留行，减少动态效果时立即移除。动画回归入口为 `tests/task-removal-motion.browser.js`。取消任务及重复下载的确认保持现有行为。

## 界面构建版本

Logo 区域和关于页版本号在 Vite 构建时注入，优先级为 `APP_VERSION` → GitHub Actions 标签事件的 `GITHUB_REF_NAME`（`GITHUB_REF_TYPE=tag`）→ `package.json` 的 version。标签如 `v1.2.3` 显示为 `1.2.3`，支持预发布版本。普通分支构建不显示分支名。

标签发布 workflow 在临时 CI checkout 中将 `package.json`、npm 锁文件、Cargo 清单/锁文件及 Tauri 配置同步到标签版本，并设置 `APP_VERSION`；不会提交这些版本变更。已构建产物不会随 Release 名称变化。

应用内容区默认禁用浏览器右键菜单；作品链接输入框提供上述编辑菜单。Logo 区域为静态展示，无按钮悬停效果。

## GitHub Actions 标签发布

[Release 工作流](../.github/workflows/release.yml) 监听 `v*` 标签推送，并严格校验正式版本 `vMAJOR.MINOR.PATCH`（例如 `v1.3.0`；暂不接受预发布或任意名称标签）。每次发布在 `docs/releases/<tag>.md` 中编写面向用户的 Markdown 更新说明，与代码和工作流一起提交，再使用该文件创建附注标签并推送：

```powershell
git tag -a v1.3.0 --cleanup=verbatim -F docs/releases/v1.3.0.md
git push --atomic origin master v1.3.0
```

工作流分别在 Windows x64、macOS Intel、macOS Apple Silicon 原生 runner 上安装 Node 24、Python 3.12 和 Rust stable，执行锁文件安装、离线测试、PyInstaller onedir 打包、隔离 cwd/PATH 的引擎启动检查，以及 Tauri 前端/桌面安装器构建。安装包包含整个 `video-engine/` 目录；Release 使用应用资源路径，不依赖构建机源码位置。

三个构建全部成功后，使用仓库自带 `GITHUB_TOKEN` 创建草稿、上传 Windows NSIS `.exe` 和两种 macOS `.dmg`，随后公开 Release。创建时通过 `--notes-from-tag` 将附注标签中的 Markdown 原样作为 Release 正文，软件“检查更新”也会读取这份正文；不再自动生成仅含提交记录的说明。务必使用附注标签，轻量标签会退回到提交信息。构建失败不发布；已有同名 Release 不覆盖。若上传中断留下草稿，先检查并删除该草稿，再重跑失败的发布 job。Actions artifacts 保留 14 天。仓库/组织策略须允许该 workflow 的 `contents: write`，无需新增发布 Token。

发布 job 必须先通过 `actions/checkout` 检出代码和标签（`fetch-depth: 0`），`--notes-from-tag` 需要本地附注标签，只有远端标签会报错。修改工作流不会改变已触发运行所使用的定义；若三个构建均成功、仅发布步骤失败，可核对原运行的 commit/tag 后，使用该运行的三份安装包和原标签说明恢复发布，不移动已有标签或替换构建来源。

上传 Release 前将安装包文件名的中文前缀替换为 ASCII `Yingqu`，避免 GitHub 规范化附件名时丢失“映取”。例如 `Yingqu_0.1.2_x64-setup.exe`、`Yingqu_0.1.2_x64.dmg`、`Yingqu_0.1.2_aarch64.dmg`；版本号、架构、安装包内容和应用显示名称保持不变。

当前未配置 Windows 代码签名；macOS 按用户决定采用 ad-hoc，不要求 Developer ID / 证书或公证。自动构建和离线启动检查不代表安装验收：全新无 Python Windows、Intel macOS 与旧系统仍待验证；Apple Silicon 本机安装启动及双平台 GUI 下载 / 重启证据见 [macOS 验证记录](macos-validation.md)。首次标签 CI 运行结果需单独确认。

## 持续集成

[CI 工作流](../.github/workflows/ci.yml) 在推送到 `master` 时自动触发，在 Windows x64、macOS Intel / Apple Silicon 上运行离线 Python/Rust 测试、Rust Clippy、打包引擎启动检查，以及前端类型检查和桌面 Release 编译（不生成安装器、不发布 Release）。同一分支的新推送会取消仍在运行的旧 CI。CI 不使用真实 Cookie 或执行平台在线下载；实际 GitHub runner 结果以 Actions 为准。

## 更新安装包缓存

安装包缓存在应用数据目录的 `updates/`。应用启动及更新包校验、保存成功后自动清理，只保留缓存中语义版本最高的一份。相同版本优先保留当前刚下载的文件，启动时则按文件修改时间选择。清理仅匹配本应用生成的 `UUID-Yingqu_<版本>_<平台后缀>` 普通文件，不递归清理目录、不跟随符号链接、不删除其他文件或进行中的 `.part`。下载失败或取消时保留之前的完整包；如已存在更高版本缓存，不提供被清理的较低版本安装包，提示重新检查更新。Windows 文件被占用时记录诊断，继续清理其他旧包，下次启动或下载成功后再尝试。

## 系统托盘与单实例

关闭主窗口会隐藏到系统托盘，已开始的解析和下载继续运行。左键点击托盘图标或选择“显示窗口”可恢复主窗口；“退出映取”先停止队列、取消并等待引擎进程回收，再退出。重复启动应用会显示、还原并聚焦已有窗口，不再打开第二个应用实例。macOS 点击 Dock 图标也会恢复窗口；这里的单实例指桌面应用，WebView 和下载引擎仍可拥有自己的辅助进程。

## 解析模块同步与补丁升级

应用标识为 `me.disign.yingqu`。首次启动在新数据目录尚不存在时，将原 `com.yingqu.desktop` 数据目录迁移到新目录，保留 Cookie、设置、历史和提交意图；已有新目录时不合并或覆盖。系统代理密码沿用原 Keychain/Credential Manager 服务命名空间，保证原凭据引用继续可用。macOS 数据库为 `~/Library/Application Support/me.disign.yingqu/yingqu.sqlite`。

固定版本离线校验及重建（PowerShell 使用 `.venv/Scripts/python.exe`）：

```sh
.venv/bin/python scripts/update_dtk_vendor.py --check
.venv/bin/python scripts/update_dtk_vendor.py
# 可改为读取已有上游 checkout 的 git blobs，不改变该 checkout：
.venv/bin/python scripts/update_dtk_vendor.py --source /path/to/upstream
```

审核新的完整提交 SHA 后同步：

```sh
.venv/bin/python scripts/update_dtk_vendor.py --ref <完整40位SHA> --update
```

同步脚本只读取 manifest 的白名单，不自动增加服务文件。先检查当前 vendor 未被手改，再从原始 git 内容重建 staging、校验/应用独立补丁、审计导入和语法；失败保留已安装源码与 manifest。版本号、原始及安装后文件 hash、补丁 hash、生成 revision 一起更新。

`vendor/dtk/` 的源码及补丁通过 `.gitattributes` 固定为 LF，避免 Windows checkout 的 CRLF 转换改变校验字节；检查器使用 UTF-8 和正斜杠清单路径。已有工作区若仍为 CRLF，应先确认没有本地修改，再重新检出该目录，不要重算 manifest 哈希来接受换行差异。

补丁冲突时先在临时上游副本检查包初始化变化，修改 `vendor/dtk/patches/desktop-imports.patch`；禁止修改 `upstream/` 已安装源码。签名和 parser 算法不改。审核补丁后使用相同 `--ref <SHA> --update` 重新生成，并检查 diff。若导入闭包增加，应明确审核 manifest 白名单、运行依赖及许可证；脚本不会自动引入未审核模块。依赖变化时生成两个跨平台锁文件并同步环境：

```sh
uv pip compile --universal python-engine/requirements.in -o python-engine/requirements.txt
uv pip compile --universal python-engine/requirements-dev.in -o python-engine/requirements-dev.txt
uv pip sync --python .venv/bin/python python-engine/requirements-dev.txt
uv pip check --python .venv/bin/python
.venv/bin/python scripts/update_dtk_vendor.py --check
cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe
.venv/bin/python -m pytest -q
cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings
npm run build
```

握手核对 manifest 的 DTK commit/version。源码变更后必须重新生成 sidecar，再构建完整应用。离线升级回归包括新版本同步、补丁冲突、未知服务依赖、本地修改保护、原图优先及 404/410 降级、网络/身份错误不降级、多文件校验和恢复。离线通过后，用有效视频与图集分别执行双平台真实冒烟，并在 Windows、macOS arm64、macOS Intel 各自验收安装包；不得用手工 fixture 或旧引擎结果替代。此次结果见 [双引擎验收](dual-engine-validation.md)。

## 运行依赖管理

入口为系统设置 → 运行依赖。缺失时由用户点击下载 FFmpeg/FFprobe、Deno 或 Fun-ASR-Nano 模型（CPU / Windows CUDA）到应用数据目录 runtime，已有系统工具优先。修改 Python 依赖模块后必须重建 PyInstaller 引擎；协议 v3、引擎 0.12.0。检查、下载、取消和平台验收见 [运行依赖说明](runtime-dependencies.md)。

## Fun-ASR-Nano 原生运行时

按[集成说明](fun-asr-nano.md)固定源码和 GGUF 权重。Windows / macOS 均构建 `llama-funasr-cli` 与 `llama-funasr-vad`。Windows 可按需安装 audio.cpp CUDA，详见 [CUDA 集成](cuda-acceleration.md)；macOS 保留 CPU。没有 Python ML 环境，旧 SenseVoice CUDA 包不参与运行。

```powershell
.venv/Scripts/python.exe scripts/build_asr.py
.venv/Scripts/python.exe -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec
.venv/Scripts/python.exe scripts/check_packaged_engine.py python-engine/dist/video-engine/video-engine.exe
.venv/Scripts/python.exe scripts/verify_transcription.py --download-models --engine python-engine/dist/video-engine/video-engine.exe
```

CMake 不在 PATH 时使用 `--cmake <绝对路径>`。构建沿用 `.local/sensevoice-build/` 的固定源码缓存，产物在忽略目录 `python-engine/asr/`；会移除该生成目录中的旧 SenseVoice helper，保留新程序、来源、许可及冒烟音频，模型不进安装包。Windows 静态 CRT、保守 CPU 指令基线及 UTF-8 manifest，macOS ad-hoc 签名。

验证入口使用 `.local/nano-verification/runtime/`，不读取平台凭据或更改用户安装。`--download-models` 下载三份约 1.2 GiB 权重，SHA-256 校验后执行真实加载/推理；短视频使用 Matroska 与 PCM16 无损音轨，避免 AAC 二次编码改变参考样本。`--long-seconds 600` 可单独测试合成长期音频，不替代自然视频准确率验证。

前端检查 `npm run build`。浏览器 IPC 隔离回归为 `tests/nano-cuda.browser.js`、`tests/transcription.browser.js`、`tests/transcription-model.browser.js`、`tests/transcript-overflow.browser.js` 与 `tests/runtime-dependencies.browser.js`，覆盖模型名称、旧 CPU/CUDA 结果、下载取消、长文本和三个视口。真实 Tauri 单独验证，不以 mock 替代。完整记录见[集成与验收](fun-asr-nano.md)。


## WB-303 片段与范围播放验证

复用既有依赖，不安装新播放器或转码工具。开发服务 `npm run dev` 后，使用现有 Playwright CLI / Edge 运行 `tests/library-clips.browser.js`；该脚本用合成 IPC 和真实媒体解码，包含三视口、无时间文案手动标记、固定版本引用、草稿保护、错源与缺源，以及正 PTS MP4。特殊样本来自 WB-302 的 `scripts/verify_transcript_clock.py` 矩阵，需先按 WB-302 流程生成 `.local/wb302/clock-matrix/mp4-origin.mp4`；其可播放上限是动态值。截图保存在 `output/playwright/`。

```powershell
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib library:: -- --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib clip_events_restart -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
playwright-cli -s=wb303 run-code --filename tests/library-clips.browser.js
```

原生验证复用 `scripts/p1a-acceptance.ps1 -Scenario upgrade -RunId wb303-final -BuildOnly` 的带标记独立 profile，只能对该合成 profile 运行 `tests/library-clips.native.js`；脚本先校验下载目录属于 wb303-final，再重建该 profile 的测试收藏。特殊第二文件必须在首次迁移前准备并同步合成旧任务的登记字节数，不能在替换测试视频后仍使用旧指纹。只给该测试进程设置 WebView2 9333 调试端口，经 CDP 操作真实生产 UI / IPC / SQLite / asset protocol，测试后结束该实例及调试端口，不对日常程序开放调试。原生用例依赖 Windows WebView2，不以浏览器 mock 或 CDP 连接本身当作 macOS / 安装包验收。实际操作与限制见[第二十一批记录](workbench-knowledge-base-validation.md#第二十一批wb-303-片段收藏与范围播放)。

macOS Apple Silicon 片段矩阵使用 `scripts/p1a-acceptance-macos.py` 的独立 `macos-b36 / upgrade` 标记 profile；通过 CUA 操作真实 WKWebView / Tauri / SQLite / asset protocol，含正 PTS、文件变化、双草稿、三视口和两种原生备份恢复。范围收缩 / 提前结束另外使用临时可见控件的 WKWebView 事件夹具，IPC 为合成 fixture，不等同真实文件自然触发；验收源码保留在本地证据目录，仓库临时页已移除。实际命令与证据范围见[第 36 批记录](workbench-knowledge-base-validation.md#第-36-批macos-片段收藏与范围播放原生验收)，不代替 Intel 或正式安装包验收。

## WB-304 关键帧验证

复用现有 FFmpeg、Pillow 与已锁定 Rust png 0.18.1，重建 engine 0.17.0 后运行：

```powershell
.venv/Scripts/python.exe -X utf8 -m pytest tests/test_keyframes.py -q
cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib keyframe
npm run build
cargo clippy --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
```

`tests/library-keyframes.browser.js` 使用现有知识库浏览器 fixture；`tests/library-keyframes.native.js` 仅连接带标记的 wb304-final 隔离 profile / WebView2 9334，调用真实 IPC，不适用于日常 profile。`src-tauri/examples/keyframes_acceptance.rs` 使用显式配置的合成源与已标记全新目录验证源码 / 打包引擎、SQLite、重开和无媒体备份恢复；不代替 GUI 或安装包验收。具体命令、输入及本轮证据见[第二十二批记录](workbench-knowledge-base-validation.md#第二十二批wb-304-关键帧提取与资源管理)。Windows TEMP/TMP 不可写时使用工作区临时目录；不弱化取消 / 清理断言。

macOS Apple Silicon 原生专项使用 `scripts/p1a-acceptance-macos.py --run-id macos-b37 --scenario empty --viewport 1280x800` 创建独立标记资料库，复用已打包引擎与本机 FFmpeg，通过 CUA 验证真实 Tauri / PNG / 子进程生命周期和两种恢复。使用 24 秒 4K 合成视频留出真实取消 / 退出观察时间，不用测试进程冒充 FFmpeg；相关 Python / arm64 Core 用例与原生证据分别报告。三窗口补验预览关闭后的按钮焦点，见[第 37 批记录](workbench-knowledge-base-validation.md#第-37-批macos-关键帧原生验收与预览焦点修复)。

## WB-305 同步阅读验证

```powershell
npm run build
playwright-cli -s=wb305 run-code --filename tests/library-sync.browser.js
playwright-cli -s=wb305 run-code --filename tests/library-sync.native.js
```

浏览器脚本使用知识库 fixture 的 seedTiming 和真实媒体，先在运行开发服务的工作区生成 `.local/wb305/long.webm`（FFmpeg lavfi color、160×90、5 fps、60s VP8）；正 PTS 样本沿用 WB-302 clock matrix。不要在同一个 CLI 浏览器并发运行多份测试，也不要在验证中改动 Vite 监听文件导致整页重载。

原生脚本仅连接带标记的 `wb305-final` 合成 profile / WebView2 9335。使用 `scripts/p1a-acceptance.ps1 -Scenario upgrade -RunId wb305-final -BuildOnly`，先运行一次完成旧库迁移并在空闲时停止该测试进程；再运行 `tests/fixtures/seed_sync_profile.py`，重启后验证。该脚本只写确认为当前工作区的标记 profile，拒绝重复种入；依赖 `.local/wb305/natural-sync.mp4` 和 WB-301 留存的真实 VAD cues，不能对日常 profile 使用。

本轮 natural-sync.mp4 为 WB-301 的约 297.586 秒测试 WAV 与 160×90 / 5fps 合成灰色视频复用封装，79 段时间和正文直接复用原测试 CUDA 输出；没有重新执行 ASR，也不据此验收原视频镜头与文字逐帧对应。短样本为合成 MP4 测试媒体，具体原生 / 浏览器及回归结果见[第二十三批](workbench-knowledge-base-validation.md#第二十三批wb-305-视频与文案同步阅读)。

macOS Apple Silicon 同步阅读原生专项使用独立 `macos-b38 / empty` profile。按既有 `seed_sync_profile.py` 方式，只在停机且 marker / 工作区 / 空库匹配后准备本地 fixture；本轮将已有 CPU 输出与明确合成的 300 秒 / 150 段 UI 时间分开留证，实际播放 / 键盘 / 来源变化均经 CUA 操作 Tauri WKWebView。现有 Windows CDP 脚本不用于 macOS；夹具不作为 WB-301 长自然语音验收，详见[第 38 批记录](workbench-knowledge-base-validation.md#第-38-批macos-同步阅读原生验收)。

## WB-306 拆解与来源验证

```powershell
cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib analysis_tests -- --test-threads=1
cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib -- --test-threads=1
cargo clippy --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
npm run build
playwright-cli -s=wb306 run-code --filename tests/library-analysis.browser.js
playwright-cli -s=wb306 run-code --filename tests/library-analysis.native.js
```

沿用已有串行核心测试约定与工作区 TEMP/TMP，避免大量 Windows 子进程并发引起 TDL fixture 超时和日志读取竞争；不更改超时或弱化断言。浏览器使用合成笔记及真实媒体 / PNG，原生仅连接 `scripts/p1a-acceptance.ps1 -Scenario upgrade -RunId wb306-final -BuildOnly` 的带标记测试 profile，WebView2 调试端口 9336。原生通过生产命令创建引用 / 拆解并实际提取 PNG，不使用日常知识库或模型凭据。

`tests/library-analysis.restart.js` 在空闲重开前后只读保存笔记快照，比对完整 ID / 创建来源 / 模型 / 修订 / 资源引用 / 正文；这是空闲强制重开测试，不代替优雅退出。旧文案 5 组、失败恢复 9 组、备份 4 组、片段 9 组回归分开运行，避免共用 CLI 会话并发和 Vite 重载。新版 schema v7 可继续使用已有 MCP / 同步阅读合成样本工具。Apple Silicon 可用上文 macOS 隔离运行器及 `--viewport` 顺序切换三个窗口，复用已保存的片段 / PNG，从原生表单创建人工及显式合成 AI 手动录入拆解；两种备份分别恢复到独立 `restore` 空库，比较 Note / analysis、固定引用、PNG 和媒体哈希。Apple Silicon 结果见[第 32 批记录](workbench-knowledge-base-validation.md#第-32-批macos-结构化拆解原生验收)，Intel 仍为 UNAVAILABLE。具体结果见[第二十四批](workbench-knowledge-base-validation.md#第二十四批wb-306-结构化文案和制作手法拆解)。

### WB-307 跨视频对比验证

沿用已有环境与 CLI，不安装工具。先完成 `npm run build`，启动 Vite 后使用独立 Edge 会话执行 `tests/library-comparison.browser.js`，覆盖三个规定视口、逐结论来源、未知维度、草稿与失败恢复。`tests/library-comparison.native.js` / `restart.js` 仅连接 WB307 标记 profile 的 9337 端口。原生构建使用 `scripts/p1a-acceptance.ps1 -Scenario upgrade -RunId wb307-final -BuildOnly`，不得对日常 profile 执行。核心串行库测试、Clippy 和 Rustdoc 沿用上节命令。避免在旧测试 EXE 运行期间重建同名测试二进制；Windows 链接器会拒绝覆盖。

```powershell
cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib comparison_tests -- --test-threads=1
playwright-cli -s=wb307 run-code --filename tests/library-comparison.browser.js
playwright-cli -s=wb307 run-code --filename tests/library-comparison.native.js
```

macOS 使用 `scripts/p1a-acceptance-macos.py --run-id <隔离标识> --scenario upgrade --viewport 1280x800` 创建 / 重建带 marker 的原生 profile；同一 run-id 保留资料，退出后再重建窗口。用原生 WKWebView 验证 2 / 4 视频和未知资料、逐结论来源、来源修订后的历史摘录、范围播放及键盘。另在全新 `restore` profile 验证含 / 不含视频备份；只读 SQL 核对比较 / 笔记 / 拆解行及媒体哈希。Apple Silicon 结果见[第 33 批记录](workbench-knowledge-base-validation.md#第-33-批macos-跨视频对比原生验收)；Windows CDP 脚本不作为 Mac 原生结果。定向核心命令为 `cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib comparison_tests -- --test-threads=1`。

### WB-308 研究专题验证

沿用已安装工具与独立 profile，资料为合成样本。`npm run build` 后启动 Vite，使用已有 Edge / Playwright CLI 跑 `tests/library-topics.browser.js`；真实 Windows 另用 `scripts/p1a-acceptance.ps1 -Scenario upgrade -RunId wb308-final -BuildOnly`，9338 调试端口只供 `tests/library-topics.native.js` / `restart.js` 连接本轮标记程序。不要对日常 profile 执行。核心保持工作区 TEMP/TMP、串行全库测试，不同时重建正在运行的测试 EXE。

```powershell
cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib topic_tests -- --test-threads=1
playwright-cli -s=wb308 run-code --filename tests/library-topics.browser.js
playwright-cli -s=wb308 run-code --filename tests/library-topics.native.js
```

macOS 使用已有 `scripts/p1a-acceptance-macos.py` 的带 marker 合成 profile，顺序重建三个规定窗口并以真实 WKWebView 操作；另建空 `restore` profile 验证含 / 不含媒体备份。核对三类成员、正文先保存、去重 / 键盘排序、来源往返、当前笔记与比较固定摘录、媒体缺失及实际范围播放；SQL 只读核对专题 / 成员的 ID、修订、顺序与来源行，媒体另验哈希。Apple Silicon 结果见[第 34 批记录](workbench-knowledge-base-validation.md#第-34-批macos-研究专题原生验收)。定向核心命令为 `cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib topic_tests -- --test-threads=1`；Windows CDP 脚本不替代 Mac 原生操作。

### WB-309 选题与草稿验证

沿用既有工具，先 `npm run build`，Vite / Edge 的独立会话执行 `tests/library-ideas.browser.js`。真实 Windows 使用 `scripts/p1a-acceptance.ps1 -Scenario upgrade -RunId wb309-final -BuildOnly` 和 9339 调试端口，`tests/library-ideas.native.js` / `restart.js` 仅连接带 WB309 marker 的合成 profile。不要使用日常知识库。核心测试保持工作区 TEMP/TMP 和串行模式，不重建正在运行的同名测试 EXE。

```powershell
cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib idea_tests -- --test-threads=1
playwright-cli -s=wb309 run-code --filename tests/library-ideas.browser.js
playwright-cli -s=wb309 run-code --filename tests/library-ideas.native.js
```

macOS 使用已有 `scripts/p1a-acceptance-macos.py` 的带 marker 合成 profile 和真实 WKWebView，分别验证三个灵感入口、观点 / 草稿 / 三类参考、来源往返、三窗口 / 键盘及重启；两种备份恢复到独立空库。CAS 验收可在确认 marker、工作区和本轮选题 ID 后，由独立连接调用生产 `library::edit_idea` 提交下一修订，再在原生旧编辑器保存；不得直接改 revision、伪造错误或操作日常库。临时验收程序退出并清理，证据保留源码、前后状态及原生完整副本。定向核心命令为 `cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib idea_tests -- --test-threads=1`。Apple Silicon 结果见[第 35 批记录](workbench-knowledge-base-validation.md#第-35-批macos-选题与草稿原生验收)。

### WB-310 受控 MCP 写入验证

授权契约见[受控写入](mcp-write-tools.md)。仅使用已有 SDK 和带 marker 的合成资料库，不使用日常 profile 或模型账号。以下 `--writes`、`--resources` 和浏览器权限检查适用于当前版本。旧 WB-310 原生写入运行器包含已退出默认工具发现的选题 / 草稿流程，仅供历史追溯，不能用于当前版本整体验收；不再复制命令适配程序。当前原生 HTTP 读写授权验证见 [HTTP 验证记录](mcp-http-validation.md)。该验证不等于安装包或 macOS 验收。

```powershell
cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib mcp_write -- --test-threads=1
cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib core_ipc -- --test-threads=1
cargo build --offline --manifest-path src-tauri/Cargo.toml --features mcp --example mcp-ipc-fixture
node scripts/verify-mcp-adapter.mjs --writes
node scripts/verify-mcp-adapter.mjs --resources
playwright-cli -s=wb310 run-code --filename tests/mcp-writing.browser.js
```

### WB-312 片段、图片与模板验收

复用已有官方 SDK、Rust、Node、Python / Pillow 和隔离 Core fixture，无新增工具安装：

```sh
cargo build --locked --manifest-path src-tauri/Cargo.toml --features mcp --example mcp-ipc-fixture
cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib mcp_ -- --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib mcp_read::media -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
node scripts/verify-mcp-adapter.mjs --media
node --test tests/mcp-client-checks.test.mjs
.venv/bin/python scripts/update_mcp_licenses.py --check
```

`--media` 在全新 `.local/wb312/run-*` 创建带 marker 的合成资料库：原文固定版本、一个片段、两张 16×16 纯色 PNG。Streamable HTTP / 内部 IPC 验证输入 Schema、分页、图片块及像素解码、仅元数据、采样范围、未知 / 跨资产 ID、明确不支持视频 / 音频、三个 Prompts 与 Tools-only 流程，并复用原有服务关闭 / EOF / 应用退出检查。不把无效 MP4 哨兵当作可播放视频、真实抽帧或 ASR 样本。

原生复验使用新的 marker profile 和仅合成资料。`capture.json` 沿用 WB-208 的实际原生配置形状，`expected.json` 必须包含 `{"purpose":"wb312-synthetic"}`；原生界面核对固定版本与 PNG，不能对日常库或真实资料执行。复用现有客户端运行器：

```sh
node scripts/verify-mcp-client.mjs '<本机 Codex 原生程序>' '<capture.json>' '<expected.json>' --media
```

默认不进行模型推理。只有明确授权使用账号及发送这些合成图片 / 元数据后，才添加 `--allow-account`；不会读取或复制认证文件。`--media` 的模型 turn 必须实际调用 get_clip / get_keyframes、接收两个 image 块、识别正确主色并返回精确 PTS / 采样范围，明确视频 / 音频未观察；结果保存于 `.local/wb312/client-*`。控制终端仍按提示配合原生关闭 / 开启服务、退出及重启，直到 summary / 自然退出都通过。

真实媒体分析质量、正式安装包和各平台独立记录，不以纯色图片识别通过替代 WB-313。Base64 复用已锁定的 0.22.1，新增直接依赖声明与完整许可，不引入新版本或媒体框架。契约与限制见[片段、图片与模板](mcp-media-tools.md)。

### WB-313 真实视频组合验收

复用上述构建、媒体读取及受控新增写入，不新增数据库格式或单独的任务系统。每轮使用 `scripts/p1a-acceptance-macos.py` 创建带 marker 的新空库，通过原生导入实际视频、CPU 转录、收藏片段、采样与结构化拆解。记录输入 SHA-256、固定 document_id / version、片段、原始 PTS / time_base 和六张保存 PNG 的哈希；没有识别文案的资料保留未知。

`expected.json` 的 purpose 为 `wb313-real`，assets 数组包含实际 id、document（无文案为 null）及 frames（含 id、file_id、clip_id、batch_id、原始 PTS / time_base、采样范围和原尺寸）。`capture.json` 来自本轮原生设置的通用 JSON，沿用 WB-208 的 `{status,json,info}` 形状。只对已明确授权的输入及隔离库执行：

```sh
node scripts/verify-mcp-client.mjs '<本机 Codex 原生程序>' '<capture.json>' '<expected.json>' --p2 --allow-account
```

`--p2` 不与 `--media` 混用。省略 `--allow-account` 只验协议和生命周期；使用账号及发送真实正文 / 图片必须有明确授权，既有合成资料授权不自动扩展。模型 turn 仅准只读 MCP Tools，必须实际接收全部六张图、读取固定原文和已有笔记，明确预览、采样及音视频未观察的限制。`scripts/mcp-p2-checks.mjs` 检查实际调用、图片块、固定引用和报告，然后运行器经现有 RPC 追加两条 general AI 笔记及一个带双片段参考的选题；这属于运行器转交模型输出写回，不等于模型自主调用写工具。桌面写授权须开启；不覆盖人工资料或原文。输出与调用记录保存在 `.local/wb313/client-*`，语义仍须另行复核。

按终端提示经原生界面关闭 / 开启服务、退出 / 重启，确认授权复位及自然退出。最终资料通过两种原生备份恢复到新的空库，核对正文、来源、固定版本、PNG / PTS、双片段参考及媒体哈希。Source 路径和可用媒体 mtime 按恢复契约重定位，不要求绝对路径相同。schema11 / writer9 / reader1～9 不变，`DESIGN.md` 的已有原文 / AI 来源、草稿与参考分区继续适用。

SDK、模型实际输入、文本语义复核、人工视听和各平台分开记录。人工只试听短段时不得扩写为全片准确；图片模型不能声称观察过连续运动、转场或声音。大图预览复用锁定的 image 0.25.10（仅 PNG），不会修改库内原图；第 40 批纯色图片记录仍为独立历史证据。当前批次结果见[第 41 批记录](workbench-knowledge-base-validation.md#第-41-批wb-313-真实视频学习到创作组合验收)。

### WB-311 MCP 转录长操作验证

契约见[转录长操作](mcp-transcription-operations.md)。使用工作区可写 TEMP/TMP 和现有 Python / SDK，不安装模型或使用账号。--operations 使用带 marker 的合成 Core，测试引擎显式UTF-8协议并生成可核对的进程 / 暂存标记；启动夹具首次 status 允许30秒准备时间，实际产品 IPC 交换仍为5秒。core/client所有操作仍有界。

```powershell
cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib mcp_operations_api -- --test-threads=1
cargo build --offline --manifest-path src-tauri/Cargo.toml --features mcp --example mcp-ipc-fixture
node scripts/verify-mcp-adapter.mjs --operations
```

Windows 原生使用 p1a-acceptance 的 upgrade / wb311-final marker、9341调试端口，HTTP 服务内置于该程序。node scripts/verify-mcp-native-operations.mjs 检查真实命令与固定操作 ID；原生程序闲置重启后以 --restart 检查状态保留与授权关闭。未满足既有识别依赖时，不把真实终态 failed 或取消视为识别质量通过。

## 轻量素材库与视听验证

本轮契约与状态见[验收记录](material-library-validation.md)。基本门禁为 npm run build，以及 desktop,mcp feature 下的 --lib 单线程测试和 --all-targets Clippy -D warnings。

```powershell
cargo build --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --example mcp-ipc-fixture
node scripts/verify-mcp-adapter.mjs --av
node scripts/verify-mcp-adapter.mjs --scope
node scripts/verify-mcp-adapter.mjs --resources
node scripts/verify-mcp-adapter.mjs --writes
node scripts/verify-mcp-adapter.mjs --operations
node scripts/verify-mcp-adapter.mjs --media
node --test tests/material-model-checks.test.mjs tests/mcp-client-checks.test.mjs
.venv/Scripts/python.exe -m pytest -q tests/test_audio_excerpt.py tests/test_keyframes.py tests/test_transcription.py tests/test_transcript_timing.py tests/test_mcp_acceptance.py
```

SDK 使用自动创建的隔离合成库，AV 模式验证真实 PNG / WAV。真实 Codex 可在 --av 后增加 --codex 原生程序路径；只有用户明确授权真实账号模型调用时加 --allow-account。使用 --materials 的真实客户端验收由运行器传入隔离配置，不改日常 MCP 设置，不读取或复制凭据。历史 --p2 选题写回入口已明确退役；Windows 源码 GUI、macOS 原生、模型实际听音及安装包都须另行记录，不能由 SDK 成功替代。

### 本机 Streamable HTTP MCP

当前 MCP 随桌面进程运行，不再构建或分发 `yingqu-mcp` 命令程序。设置中启用服务并复制 `http://127.0.0.1:17653/mcp`；客户端使用 Streamable HTTP URL 配置。旧 stdio / command / args 接入记录仅作历史证据，不能用于当前版本。`desktop` feature 包含 `mcp`，开发与验收程序也可直接开启 HTTP 服务。

```powershell
cargo build --locked --manifest-path src-tauri/Cargo.toml --features mcp --example mcp-ipc-fixture
node scripts/verify-mcp-adapter.mjs --library
node scripts/verify-mcp-adapter.mjs --writes
node scripts/verify-mcp-adapter.mjs --resources
node scripts/verify-mcp-adapter.mjs --operations
node scripts/verify-mcp-adapter.mjs --scope
node scripts/verify-mcp-adapter.mjs --av
cargo test --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings
```

合成 Core 使用带标记的独立资料库和 OS 分配的本机端口；日常桌面端口不用于自动化测试。HTTP 专项测试验证非本机 Host/Origin、过大请求、端口占用回滚、慢请求关闭、写授权变化和真实 FFmpeg 断连回收。设置页浏览器与原生脚本分别为 `tests/mcp-settings.browser.js` 和 `tests/mcp-settings-native.browser.js`。普通 HTTP、SDK、Codex、原生窗口与安装包结果分别记录，见[HTTP MCP 验证](mcp-http-validation.md)。
