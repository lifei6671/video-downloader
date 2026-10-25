# Fun-ASR-Nano 单视频文案提取

2026-10-23 用户要求以 Fun-ASR-Nano 替换 SenseVoiceSmall。当前协议 v3、内置引擎 0.12.0。Windows 新增可选 [audio.cpp CUDA 与独立 VAD](cuda-acceleration.md)，macOS 保留 CPU。首次提取或系统设置 → 运行依赖提供一项“语音转录模型 Fun-ASR-Nano”，按需下载，查看、复制和 TXT 导出沿用现有流程。

## 运行与安装

- Windows x64、macOS Intel / Apple Silicon 使用官方 C++ `llama-funasr-cli` CPU 程序，不安装 FunASR Python、PyTorch、CUDA Toolkit 或监听服务。
- CPU 路径继续使用原 CLI；Windows 同卡提供 CPU/CUDA 版本选择。CUDA 使用 audio.cpp 专用模型及原生 DLL，详见 [CUDA 集成](cuda-acceleration.md)，不沿用 SenseVoice CUDA ZIP。
- 选择依据：[官方 GGUF](https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-GGUF) 与[固定上游 CLI](https://github.com/modelscope/FunASR/blob/fbbfe01800a8c4c9103cb2150fb1a5d06cc64758/runtime/llama.cpp/fun-asr-nano/funasr-cli/funasr-cli.cpp) 已有原生 CPU 实现；此前本机同视频测试三处“降智”全部正确，无需新 ML 依赖或自研识别算法。单个样本不代表普遍准确率。
- `scripts/build_asr.py` 校验固定 FunASR 与 llama.cpp 源码归档，构建 CPU helper 和独立 VAD helper，保留静态 CRT、跨平台基线和许可；模型不随安装包分发。Windows 使用 `vendor/asr-windows.manifest` 的 UTF-8 activeCodePage。
- 权重保存到 `<应用数据目录>/runtime/fun-asr-nano/`，合计 **1275804800 bytes，约 1.2 GiB**。依赖 ID 为 `fun-asr-nano`，附加可选 `variant=cpu|cuda`，省略为 CPU；不接受旧 `sensevoice` / `sensevoice-cuda` ID。CUDA 模型单独保存，CPU 三文件保持原样。

## CPU 固定权重

| 文件 | revision | bytes | SHA-256 |
| --- | --- | ---: | --- |
| `funasr-encoder-f16.gguf` | `46e849502a867080d66d351b8dfb1018b607e509` | 469331008 | `f92f91d01a24fbed6c863495b2ee8c6a6788144a02858b75743f0946668de8a2` |
| `qwen3-0.6b-q8_0.gguf` | 同上 | 804753280 | `819f385dc0e035dccc3d9e7edaf6b7b044b8ba7ace63cbcbf84c7e397eecbf27` |
| `fsmn-vad.gguf` | `6840bae4c5c92ee8c04faaf4db23dd0105098d7f` | 1720512 | `1270f2559c495f4e7b6e739541151027d360761a3fda43fc147034f5719f5479` |

前两份来源 `FunAudioLLM/Fun-ASR-Nano-GGUF`，VAD 来源 `FunAudioLLM/fsmn-vad-GGUF`，均为 Apache-2.0。生产表位于 `python-engine/runtime_assets.py`；来源及许可见 [UPSTREAM](../vendor/UPSTREAM.md)。

安装使用原有系统代理和受限 HTTPS 主机，不携带平台 Cookie 或平台代理。逐文件流式验证大小与 SHA-256，全部成功后执行真实离线冒烟，写入 manifest、来源及许可，再整体替换。任一文件损坏、推理失败或提交后不可用均保留旧 Nano。取消/退出回收操作进程与暂存，不提交半套模型。模型安装和转录互斥，其他视频下载可继续。

原 `runtime/sensevoice/`、`runtime/sensevoice-cuda/` 及旧选择文件保留，不会被当作 Nano 已安装，也不会让 Nano 选择 CUDA。升级不隐式删除用户模型或重算文案。

## 转录与旧结果

Rust 根据已完成任务路径启动独立 sidecar；FFmpeg 只读取原视频，生成临时 16 kHz 单声道 PCM16 音频。CPU 固定 CLI 参数为 `--enc <encoder> -m <decoder> --vad <vad> --vad-maxseg 30000 -a <wav>`；CUDA 使用 [独立 VAD 与 audio.cpp 批处理](cuda-acceleration.md)。不接受任意前端程序或模型路径。Python 负责进程和结果文件，stdout 保留 JSONL 协议，全文由 Rust 校验后入库。

保留转换最多 1800 秒、识别最多 `max(600, 3×音频时长+120)` 秒、每 5 秒心跳、最多 16 MiB 文本/日志、取消和退出清理。模型下载最多 30 分钟，Rust 整次安装最多 45 分钟；FFmpeg/Deno 原超时保持。

引擎 0.14.0、协议仍为 v3，`transcription_progress` 增加 `loading_model` 阶段和可选 `completed_segments` / `total_segments`。CPU helper 在模型加载后、VAD 分段及每个实际识别窗口完成时输出并刷新专用 `YINGQU_ASR_PROGRESS` 记录；短于原生最小帧长而跳过的窗口不计入分母。`scripts/build_asr.py` 对固定源码仅添加此通知，不改推理、分段算法或正文输出；SOURCE.json 记录 `progress_version=1`，打包要求重建该 helper。CUDA 使用固定 audio.cpp 的准备完成记录及逐段结果文件，整个批次仍只加载一次模型，不按用时估算进度。原生日志和正文不进入进度事件。

界面在模型加载时仅显示加载状态，识别时显示“已识别 X / Y 段”和完成片段比例；这是识别片段的完成比例，不是耗时百分比或剩余时间预测。Rust 校验计数有序、总量稳定，再广播任务快照。关闭再打开弹窗保留当前进度，开始新操作、取消或重启中断时清空临时计数；仍以结果校验和 SQLite 提交后为成功。

CPU 新结果保存名称 `Fun-ASR-Nano`、上述模型 revision、pipeline `fun-asr-nano-cpu-fbbfe018-v1`、backend `cpu`；CUDA 保存其实际模型 revision、pipeline 和 backend。SQLite 结构不变，旧结果缺少 `model_name` 时按 SenseVoiceSmall 展示，保留原 CPU/CUDA 标识。旧文案在源视频缺失后仍可复制/导出；失败、无语音或取消保留旧正文。重新提取成功后才替换正文与模型身份。

TXT 继续输出 UTF-8 BOM 与平台换行，保留全文，原子写入且禁止覆盖原视频或其别名。界面保留长文案独立滚动、固定标题/底部操作、连续长串换行和键盘路径。

正文使用只读 shadcn Textarea，可聚焦、鼠标选取，支持系统全选和复制快捷键（Windows Ctrl+A / Ctrl+C，macOS Cmd+A / Cmd+C）；不能修改已保存文案。运行中的进度快照不反复读取或清空正文，保留“复制全文”和“导出 TXT”操作。

加载提示行使用 `overflow-clip` 限制旋转图标的几何溢出，避免读取文案、提取过程中出现短暂滚动条；正文容器仍按内容高度自动滚动。

## 原 CPU 替换验证（历史记录）

下表为此前 CPU 替换的验收，不代表本轮 CUDA 验收。本轮见 [CUDA 验证](nano-cuda-validation.md)。

本次证据目录为 `.local/nano-delivery/`。以下均为本轮实际执行结果，未提交或推送：

| 状态 | 验证 | 结果与证据 |
| --- | --- | --- |
| PASS | Python 全量 | 388 passed；`python-full.log`，另有 30 项针对性回归 `python-targeted.log` |
| PASS | Rust 全量与 clippy | 110 passed、2 ignored；`rust-tests.log`、`clippy.log` |
| PASS | 前端与 Windows 构建 | `frontend-build.log`、`native-helper-build.log`、`pyinstaller.log`、`packaged-check.log`、`desktop-build.log`、`desktop-final-build.log`；引擎握手 0.11.0 |
| PASS | 模型完整性与安装 | 复用此前完整下载的官方三份文件作为缓存输入，生产安装器完整校验 SHA-256、真实离线冒烟并提交；`cached-install.json`、`user-install.json`。这不是一次新的完整网络下载 |
| PASS | 实际下载地址 | 生产 HTTPS opener 逐文件跟随受限重定向并读取前 1024 字节，响应长度与固定表匹配；`network-origin-check.json`。不将它称为本轮完整网络下载 |
| PASS | 原生与打包短视频 | 无损 3 秒视频识别“开放时间早上九点。”，含转换/识别约 2.42 秒；打包 JSONL 输出相同，原视频 hash 不变；`real-short-lossless.log` |
| PASS | 浏览器 UI | Edge headless 22 组，覆盖 1440×900、1280×800、1024×680、独立下载/取消、旧 CPU/CUDA 模型标识与失败恢复提示、重新提取、复制/导出和长文案滚动；`browser-results.json` 及截图已查看。未测 headed 原生箭头点击，滚动条样式未改 |
| PASS | 真实 Tauri | 隔离实例读取同一条约 298 秒原始下载视频，完整转录约 194 秒（按 Rust 开始时间估计），1816 字符，三处“降智”正确；模型/修订/CPU 后端落库、重开、取消保留正文、依赖可用；`native-result.json`、全文和截图已查看 |

真实 Tauri 本轮仅对复制/导出按钮作可点击命中验证，未写真实剪贴板或操作保存对话框；全文一致性与导出格式由浏览器 fixture、Rust 原子导出测试分别验证，不冒充原生保存框验收。隔离测试只修改已知 `asr-native-test` 记录，完成后已恢复原始任务/文案快照，原视频 SHA-256 不变。当前用户目录只新增经校验的 Nano 模型，不覆盖旧模型或用户文案。

质量观察：原三秒样本经 AAC 再编码时 Nano 输出“开饭时间早上九点”，原始 WAV/无损视频输出“开放时间早上九点”。集成冒烟使用无损音轨以隔离转封装影响；没有修改识别文本或添加纠错，真实下载视频另行验证三处目标词。Nano 仍可能存在其他错字，单个样本不代表全文准确率。

macOS Intel / Apple Silicon 实际构建、安装和推理：**NOT_RUN**，本机 Windows 不替代。全新无 Python Windows 安装：**NOT_RUN**，仍属 M3。该历史阶段的 Nano GPU 与全文人工准确率：**NOT_RUN**，不以短语音冒烟或单视频目标词正确替代。

### 加载状态滚动条回归（2026-10-23）

证据位于 `.local/transcript-scrollbar-flicker/`。`tests/transcript-loading.browser.js` 在 1440×900、1280×800、1024×680 连续采样两个完整旋转周期，覆盖读取、识别两种状态及长标题。修复前可复现 1–3 px 动画溢出，新增测试失败；修复后每帧均无溢出，键盘取消可用。现有提取流程 5 组、长文案与连续长串滚动 6 组均 **PASS**，包含完整复制/导出和固定底部操作。`npm run build` **PASS**。本次样式修复的真实 Tauri 窗口与 macOS 运行 **NOT_RUN**，不沿用上表模型替换时的原生验收结果。
