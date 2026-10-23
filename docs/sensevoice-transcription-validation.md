# SenseVoiceSmall 文案提取验证

> 历史验证：当前模型已更换为 Fun-ASR-Nano，Windows 可选 CUDA，当前接口、验证及限制以 [Nano 集成说明](fun-asr-nano.md) 为准。下文保留 SenseVoice/CUDA 阶段证据，不代表 Nano 验收。

日期：2026-10-23。环境：Windows x64、现有开发机；原 CPU 验收及本次 CUDA 扩展分别记录。范围为[已批准方案](sensevoice-transcription-plan.md)的代码、回归、必要文档；本次未提交、推送、发布，也未读取平台凭据。以下技术验证不等于自然视频识别质量或其他平台安装验收。

## 已实现行为

- 已完成视频行提供提取/查看文案，复用 shadcn Dialog、运行依赖下载、原生剪贴板和保存对话框。模型与缺失 FFmpeg 都由用户点击安装，安装后不自动转录。
- 固定官方 Q8 + FSMN-VAD，native CPU helper 随应用分发，模型按需下载约 244 MiB。用户无需另装 Python/PyTorch/CUDA；无系统 Python 的全新机器安装仍单独待验收。
- Rust 单独管理一个转录操作，保留下载状态；关闭弹窗继续处理。取消/退出回收进程树，重启中断不自动重跑；重新提取失败、取消或空结果保留上次成功正文。
- 正文独立存 SQLite；复制/导出只使用已落库结果。TXT 为 UTF-8 BOM、Windows CRLF，原子写入并拒绝覆盖源视频及硬链接别名。删除记录联动清理文案，导出 TXT 保留。

## 自动回归与构建

| 状态 | 实际命令/入口 | 结果与覆盖 |
| --- | --- | --- |
| PASS | `cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe` 后 `.venv/Scripts/python.exe -m pytest -q` | 384 passed；新增长度/hash/修复失败保留旧安装、实际子进程超时回收、UTF-8/空结果、无音轨、本地 FFmpeg 参数和重定向隔离测试 |
| PASS | `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` | 109 passed，2 ignored（既有 GitHub 在线更新测试）；新增持久化/重启/删除、旧结果保留、取消/退出、互斥、导出编码/文件别名、SQLite 终态写入失败测试 |
| PASS | `cargo test --locked --manifest-path src-tauri/Cargo.toml transcription -- --test-threads=1` | 最后修改后的 5 项转录回归全部通过 |
| PASS | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | 桌面全部目标检查通过 |
| PASS | `tests/transcription.browser.js` | 5 组场景：三视口生命周期、复制/导出一致、关闭重开、迟到事件、取消保留旧文案、下载后手动开始、视频缺失仍可查看 |
| PASS | `tests/task-actions.browser.js` | 13 组既有任务行回归；新增图标不破坏三视口、键盘、复制/删除和 pending/失败行为 |
| PASS | `scripts/build_sensevoice.py` | 固定两份源码归档 SHA-256；Windows CPU 基线、静态 runtime、UTF-8 manifest，native 无 CUDA/PyTorch 依赖 |
| PASS | PyInstaller + `scripts/check_packaged_engine.py python-engine/dist/video-engine/video-engine.exe` | 引擎 0.8.0 / 协议 v3；隔离 cwd/PATH 启动，包含 helper、冒烟 WAV 与许可，不含模型权重 |
| PASS | `npm run tauri -- build --features desktop --config src-tauri/tauri.release.conf.json --no-bundle -- --locked` | 前端类型/生产构建和 Windows Release 桌面编译通过；保留既有 bundle 大小及 STATIC_VCRUNTIME 弃用警告 |

最初 Python 全量回归中 11 项 probe 测试使用了旧 0.7.5 可执行文件而失败，按开发文档重建 probe 后最终全量 384 项通过。没有降低测试或兼容旧握手来掩盖版本问题。

## 真实模型与原生桌面

使用 `scripts/verify_sensevoice.py --download-models --engine python-engine/dist/video-engine/video-engine.exe` 在 `.local/sensevoice-verification/` 隔离目录调用生产安装代码。两个模型完整字节数与 SHA-256 均匹配生产 manifest；加载/推理成功，已下载后使用本地模型，不请求平台或模型 API。单独验证含中文和空格的模型/暂存路径。

| 状态 | 实际输入 | 耗时 | native 峰值工作集 | 可观察结果 |
| --- | --- | ---: | ---: | --- |
| PASS | 官方 3 秒语音转成的 MP4 | 1.094 秒 | 274669568 bytes（约 262 MiB） | “开放时间早上9点。”；源视频前后 SHA-256 一致；打包 sidecar JSONL 路径通过 |
| PASS | 相同语音循环生成的 600 秒 MP4 | 96.75 秒 | 609083392 bytes（约 581 MiB） | 返回完整非空文案，超过原普通操作 80 秒仍正常完成；源视频 SHA-256 一致 |
| PASS | 真实 Tauri WebView + 打包 sidecar + 同一短视频 | 单独交互验证 | 未作 GUI 峰值测量 | 点击提取、显示真实结果、原生复制成功、关闭重开、SQLite completed；原下载仍 completed |

真实 Tauri 使用独立 `me.disign.yingqu.sensevoice-validation` 数据目录和固定 `asr-native-test` 记录，没有使用真实下载历史、Cookie 或代理凭据。通过 `tests/transcription-native.browser.js` 在实际 WebView 操作。实际运行发现并修复了同步 Tauri 命令缺少 Tokio reactor 的崩溃，改成异步入口后整条真实链路通过。

浏览器 mock 在 1440×900、1280×800、1024×680 检查弹窗边界、可操作状态、键盘和焦点；截图与真实 Tauri 截图保留在忽略目录 `.local/transcription-delivery/`。真实 GUI、fixture 和核心测试分别记录，不相互替代。

## 验收限制

| 状态 | 尚未执行的范围 |
| --- | --- |
| NOT_RUN | 自然长视频、粤语、英语、中英混说、BGM/纯音乐、各种静音样本的人工核听和准确率评估；10 分钟合成样本只验证资源及超时 |
| NOT_RUN | 原生保存对话框的人工选择/取消/覆盖交互；生产核心 TXT 字节、替换、禁止覆盖视频和浏览器调用已自动验证 |
| NOT_RUN | macOS Intel / Apple Silicon helper、签名、GUI 与安装；CI 配置已加入构建步骤，本次未触发远端工作流 |
| NOT_RUN | Windows 全新无 Python 系统安装包、安装后首次模型下载和断网整机使用；仍属于 M3 安装验收 |

本轮独立审查发现的 SQLite 终态落库失败导致查询仍显示 running 已修复，并以失败触发器回归验证：查询返回 failed，不提供未落库的新结果，先前成功正文仍可复制/导出。上述自动回归、构建和真实 CPU/GUI 证据均针对当前实现，不将文件存在或 `--help` 视作模型可用证明。

## 原独立 CUDA 卡片验收（2026-10-23，已被下方版本选择方案取代）

范围：[CUDA 集成说明](cuda-acceleration.md)。Windows x64，NVIDIA GeForce RTX 4070 SUPER（计算能力 8.9、约 12 GiB 显存），驱动 API 报告 CUDA 13.3；安装包要求 API 至少 13.2。当前引擎 0.9.0、协议 v3。没有更改真实用户的模型、任务、账号或 CUDA 安装；GPU 包安装和真实提取使用隔离目录。预览程序因占用构建文件退出后恢复启动。

| 状态 | 命令/证据 | 结果 |
| --- | --- | --- |
| PASS | `.venv/Scripts/python.exe -m pytest -q` | 408 passed；增加 24 项 CUDA 测试，覆盖架构/驱动、多显卡、macOS CPU 分支、独立下载、hash/缺 DLL/推理失败、提交后回滚、模型不变、自动选后端和无隐藏 CPU 重试 |
| PASS | `.venv/Scripts/python.exe -m pytest -q tests/test_cuda_runtime.py` | 最后跨平台 fixture 修正后 24 passed；安装测试显式模拟 Windows，避免 macOS 测试误用真实平台状态 |
| PASS | `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` | 110 passed、2 ignored（既有在线更新测试）；新增 CUDA backend/pipeline 持久化及旧记录兼容 |
| PASS | `cargo test --locked --manifest-path src-tauri/Cargo.toml transcription -- --test-threads=1` | 最终 6 项通过，含 CUDA 错误传播、失败保留旧文案 |
| PASS | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | 检查通过 |
| PASS | `npm run build`、`npm run desktop:build` | TypeScript/Vite 和 Windows 桌面构建通过；既有 Vite 分包/use-client 及 staticVCRuntime 弃用提示仍在 |
| PASS | PyInstaller 和 `scripts/check_packaged_engine.py` | sidecar 重建，握手 0.9.0；离线检查限制 PATH，不依赖系统 Python |
| PASS | `tests/runtime-dependencies.browser.js` | 6 组，1440×900 / 1280×800 / 1024×680；5 张独立卡、固定头部、边框/间距、单项下载、CUDA 模型前置条件及修复/取消、旧驱动/无驱动/不支持显卡提示；本次 Edge 为无头浏览器，未重复原生箭头点击 |
| PASS | `tests/transcription.browser.js`、`tests/cuda-transcription.browser.js` | 5 + 2 组；既有提取/复制/导出/取消、CUDA 实际后端、旧 CPU 结果标签和重开 |
| PASS | `.local/cuda-delivery/install-real.json` | `gh release download v1.4.11` 完整下载标准 CUDA ZIP，实际长度及 SHA-256 与固定表相同；将该缓存 ZIP 输入生产安装器，完整校验、受限解压、真实 GPU 冒烟和提交均通过。模型目录所有文件前后哈希完全一致；此项先验证缓存安装；随后另行完成生产下载器在线端到端测试，见下行 |
| PASS | `scripts/verify_sensevoice.py --download-cuda --expect-backend cuda` | 生产网络下载器真实完整下载标准 CUDA 包（412113100 bytes），经固定 hash、受限解压、真实 GPU 冒烟和整目录提交成功；已有模型完整性检查及后续 CUDA 转录通过。日志 `.local/cuda-delivery/production-download.log` |
| PASS | `scripts/verify_sensevoice.py --expect-backend cuda --engine python-engine/dist/video-engine/video-engine.exe` | 真实 3 秒语音视频，中文/空格路径，生产编排和打包 sidecar 输出“开放时间早上9点。”；短视频生产编排约 1.02 秒，native 峰值工作集约 500 MiB，原视频哈希不变 |
| PASS | `scripts/verify_sensevoice.py --expect-backend cuda --long-seconds 600` | 10 分钟重复语音合成视频约 10.27 秒，native 峰值工作集约 604 MiB，原视频哈希不变；不代表自然长视频质量或显存峰值 |
| PASS | `.local/cuda-delivery/native-result.json` | 隔离 Tauri 实例使用当前打包 sidecar，真实运行依赖卡识别 GPU/CUDA 可用，提取成功并保存 `backend=cuda`、`sensevoice-cuda-fbbfe018-v1`；关闭重开 Dialog 状态/文本保持。截图 `native-runtime.png`、`native-transcript.png` 已查看 |
| PASS | `git diff --check` 及本轮未跟踪源文件检查 | 没有新增空白/引用问题；原有未提交工作保留 |

- **NOT_RUN**：RTX 30、RTX 50/Blackwell 物理显卡、多 GPU 实机、macOS Intel / Apple Silicon 实机、全新无 Python / 无 Toolkit 系统安装、自然视频人工核听质量。Blackwell 包仅核对官方 Release 大小/摘要，未在本机执行。
- 审查与冻结证据位于 `.local/cuda-delivery/`，本轮只审 task-start baseline 之后的 CUDA 增量；此前 CPU/UI 未提交内容作为上下文保留，不归为本轮新增实现。

本轮测试创建的隔离 AppData 目录 `C:\Users\lifei\AppData\Roaming\me.disign.yingqu.cuda-validation-20261023` 在测试进程退出后尝试清理，但自动审批以“blocked by policy”拒绝递归删除，故保留。真实用户的应用目录与模型不受影响。

## 单项语音模型的 CPU/CUDA 版本选择（最新，2026-10-23）

当前有效契约以 [CUDA 集成说明](cuda-acceleration.md) 为准。用户要求 CUDA 属于语音模型，不单列为依赖；普通检测和 CPU 路径不查询 GPU，只有请求 CUDA 安装后才检测兼容性。对外为 `tool=sensevoice, variant=cpu|cuda`，引擎 0.10.0、协议 v3。旧独立 CUDA 卡片/安装 ID 已移除；上节仅保留旧版本的历史验证证据。

- **PASS**：Python 全量 415 passed；最终相关 57 项通过，覆盖首次 CUDA 合并安装、共享模型/组件复用、CPU 切换无 GPU 查询/无下载、兼容性失败零网络、加载/长度/hash/缺文件失败及多目录/版本回滚。
- **PASS**：Rust 全量 110 passed、2 既有 ignored；最终依赖操作回归验证旧 CUDA ID、非模型携带 variant、未知 variant 均被拒绝；clippy 全目标检查通过。
- **PASS**：TypeScript/Vite、PyInstaller 重建及 Windows 桌面构建；打包引擎握手 0.10.0。
- **PASS**：浏览器 6 组依赖、5 组转录及2组后端标签；1440×900、1280×800、1024×680 中仅4张依赖卡（含 tdl），语音模型同卡选择 CPU/CUDA；兼容性提示在请求 CUDA 后出现，macOS 仅 CPU。界面截图已查看，原滚动条样式未改。
- **PASS**：本机 RTX 4070 SUPER，生产安装路径复用既有完整模型与 CUDA 组件，选择 CUDA 后真实推理成功；重建后的打包 sidecar 仍输出“开放时间早上9点。”。
- **PASS**：真实 Tauri 隔离实例在同一模型卡切换 CUDA、再切换 CPU，安装事件无网络时为 installing/0字节，Rust/界面均正常处理；分别真实提取并保存 `backend=cuda/cpu`。旧 CUDA 文件留存时 CPU 选择仍实际使用 CPU。证据 `.local/asr-variant-delivery/native-result.json` 和两版卡片/文案截图已查看。
- **PASS**：本轮空白、Markdown本地引用和工作区差量检查；保留原有修改，不提交或推送。
- **NOT_RUN**：macOS/其他 NVIDIA 显卡/多显卡实机、全新系统安装及自然视频人工核听质量。新合并安装的首次缺少两种资产路径通过离线固定ZIP/模型夹具覆盖；本轮不重复联网下载已在上轮验过且未变的约393MiB包。

本轮日志及审查证据在 `.local/asr-variant-delivery/`；真实桌面验证复用此前专属隔离 AppData 目录，不创建或删除用户真实记录，也未重试此前被自动审批拒绝的目录清理。

## 长文案溢出修复（2026-10-23）

只调整文案 Dialog 的布局：内容项不被 Flex 压缩，长文本保持自然高度并在外层内容区滚动；连续无空格文本允许换行，标题/底部按钮固定，时间排列在正文之后。转录、保存、复制和导出的数据逻辑不变。

- **PASS**：`tests/transcript-overflow.browser.js` 在修复前以“text escaped its own border”失败；修复后6组通过，覆盖长中文、无空格长文本和1440×900 / 1280×800 / 1024×680，检查边框容纳全文、时间无覆盖、键盘翻页、标题/操作区固定、按钮命中与完整复制/TXT导出。
- **PASS**：`tests/transcription.browser.js` 既有5组通过；`npm run build` 类型及前端构建通过。
- **PASS**：隔离真实 Tauri 使用19800字测试文案，正文高度与内容高度一致（10042px），外层可视612px正常滚动，时间位于正文之后，底部三个操作可命中；顶部/底部截图已查看。测试只修改此前专属隔离记录，结束后恢复原快照，不读写真实用户历史或剪贴板。
- **PASS**：Windows 桌面构建、改动空白和引用检查。证据位于 `.local/transcript-overflow/`。
- **NOT_RUN**：macOS 实机；本次未改 Python/Rust 行为，不重复运行无关核心测试。
