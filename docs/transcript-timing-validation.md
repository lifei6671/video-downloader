# 文案时间来源与精度验证

本页保留 WB-301 当批调查与试验边界；后续 WB-302 已实现生产时间存储，并通过播放器实测修正非零起点的处理，当前契约见 [可追溯段落](transcript-segments.md)。本页的候选归零试验不代替后续 PTS 零点契约。

对应 [WB-301](workbench-knowledge-base-tasks.md)。最初 Windows 调查检查固定引擎、实跑 VAD / ASR 和媒体时间原点，确定 WB-302 的实施边界，当批没有修改生产转录、协议、数据库或界面。可交付方向是 **VAD 语音段级定位**。句子、词、字、逐帧字幕同步均不承诺。2026-10-29 Apple Silicon 已补长语音复验，见本页末节；Intel 无设备，仍为 UNAVAILABLE。下文“当前生产”和 Windows 试验描述保留原调查时点，不代替后续生产契约。

## 时间从哪里来

| 入口 | 实际含义 | 当前生产是否保存 |
| --- | --- | --- |
| CPU `llama-funasr-cli --srt` | 对应输入音频的 VAD 窗口起止，整数毫秒；同一窗口可有多句 | 否；生产没有传 `--srt`，仅收合并正文 |
| 独立 `llama-funasr-vad` | `start_ms end_ms`，没有正文 | CUDA 暂存 `vad.txt`，完成后清理 |
| CUDA `transcript_sXXXXXX.txt` | 按对应 VAD 裁剪顺序逐片段识别的正文 | 暂存后用换行合并；没有随正文返回区间 |
| `completed_segments / total_segments` | 进度计数，片段长短不同 | 可用于进度，不能反推时间 |
| Rust 转录完成处理 | 校验 `transcript.txt`、大小、no_speech、源文件身份和后端，再创建不可变文档 | 目前没有段落时间实体 |

CPU 来源为 [固定 FunASR CLI](https://github.com/modelscope/FunASR/blob/fbbfe01800a8c4c9103cb2150fb1a5d06cc64758/runtime/llama.cpp/fun-asr-nano/funasr-cli/funasr-cli.cpp)。`start_ms / end_ms` 直接由采样窗口换算，SRT 只改变输出格式，没有新增文本对齐；`/sil` 窗口可被省略，不能用 SRT 序号冒充全部 VAD 窗口序号。固定源码 HTTP 读取及哈希见 `.local/wb301/research/source-check.json`；本地构建只加进度日志，见 `scripts/build_asr.py`。

VAD 状态机见 [固定 funasr_vad.h](https://github.com/modelscope/FunASR/blob/fbbfe01800a8c4c9103cb2150fb1a5d06cc64758/runtime/llama.cpp/funasr-common/funasr_vad.h)：帧步长 10ms，含起点回看、终点前探、随累计时长变化的静音阈值和最大分段限制。**10ms 是计算网格，不是 ±10ms 准确度保证**。不能由整数毫秒格式推断语义句边界、词边界或实际镜头切点。

CUDA 继续使用 [audio.cpp 固定发布](https://github.com/0xShug0/audio.cpp/releases/tag/v0.9.2-drama3-base-small)，源码 `50749561`；本轮实际 framework 日志报告 `runtime.model.supports_timestamps 0`。其文本可绑定实际裁剪窗口，但不能从模型输出获得逐字时间。模型 / pipeline 固定修订见[已有集成](cuda-acceleration.md)。

## 实跑设计与证据

不安装工具或模型、不使用云端模型或真实凭据。复用仓库本地的 CPU / CUDA 已校验运行目录，Windows RTX 4070 SUPER，模型与二进制实际哈希见 `.local/wb301/environment.json`。新增运行器只在新的 `.local` 子目录生成产物，输入只读，拒绝覆盖已有输出目录。

`scripts/verify_transcript_timing.py` 对每个样本运行独立 VAD、CPU `--srt` 与实际生产 CUDA 编排，逐项核对窗口、裁剪的真实采样帧、识别文件顺序及合并结果。短样本额外运行当前 CPU 编排，确认返回仍只有全文等字段，没有 `segments`。测试构造已知静音位置，但这些位置是**素材摆放边界**，不是人工标注的音素 / 词边界，不用它们计算词级准确率。

| 样本 | 组成与用途 |
| --- | --- |
| `short-zh` | 3 秒官方中文人声；复用已有冒烟 WAV，来源与许可见 [UPSTREAM](../vendor/UPSTREAM.md) |
| `pause-zh-en` | 18.476 秒，800ms 前置静音、中文、1800ms 静音、官方英文人声、1800ms 静音、中文、900ms 尾部静音；检查原始时间轴和语言切换 |
| `continuous-zh` | 中文片段无额外静音重复 22 次，共 66 秒；检查约 30 秒的长语音窗口及多句共用区间 |
| `natural-zh` | 已有隔离 ASR 验证缓存中的约 298 秒自然中文口播；本轮重新推理，不访问日常知识库或原始下载目录 |

英文来自[固定模型仓库 example/en.mp3](https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-2512/blob/272c57b82523ada6fd87095e955f8e29100979ab/example/en.mp3)，仓库声明 Apache-2.0，仅本地测试使用，不加入安装包。MP3 SHA-256 `f10378336a4e584f3f63799e62f99d5add3c2a401b51d3abe7d3a3a82f255ada`，转成 PCM16 后 WAV SHA-256 `e0e3ffa41b62117a666edd40a7479dbb07eec55e46884415de007ec5401c9e50`；下载 / 转换记录见 `.local/wb301/inputs/provenance.json`。自然口播缓存 SHA-256 `e100e05646a83c19539f8a1741aaea13bcca1927d6c5fb74e1424bf60a56ee2d`，仅作为既有隔离样本，全文人工准确率不计通过。

实跑结果见本页后续结果表和[第十九批实施记录](workbench-knowledge-base-validation.md#第十九批wb-301-文案时间来源与精度验证)。完整产物位于 `.local/wb301/run-complete/`；失败试跑独立保留，不能代替完整运行。

### 实际结果

| 样本 | 音频时长 | VAD / CPU SRT / CUDA 映射数 | 最长窗口 |
| --- | ---: | ---: | ---: |
| `short-zh` | 3000ms | 1 / 1 / 1 | 2560ms |
| `pause-zh-en` | 18476ms | 3 / 3 / 3 | 5920ms |
| `continuous-zh` | 66000ms | 4 / 4 / 4 | 30000ms |
| `natural-zh` | 297586ms（毫秒显示向下取整） | 79 / 79 / 79 | 30000ms |

共 87 个窗口；CPU SRT 与独立 VAD 的归一化起止完全一致，CUDA 87 份裁剪的真实采样帧数、区间、逐段文件顺序及合并文本核对通过。四组 CPU 本轮均无 `/sil` 省略；这不是所有输入都不省略的承诺。原始三个输入的 SHA-256 前后一致。最终主运行退出 0，`summary.json` 为 `PASSED`；这个状态仅指实验的结构与映射断言。

短中文时间为 `[420, 2980)`，全文“开放时间早上九点。”。混合样本的三个区间为 `[1220, 4030)`、`[6400, 12320)`、`[15000, 18460)`；最后中文素材放置于 `[14576, 17576)`，VAD 尾部包含了 884ms 的已知追加静音，说明范围有余量，不能当作末字边界。英文片段正确读出英文句子，但两处中文被识别成“太放时间早上九点。”，正文正确率不能由时间映射通过推导。

66 秒样本中的 `[420, 30420)` 是整整 30 秒窗口，CPU 返回 99 个字符，CUDA 只返回 9 个字符，出现重复语音压缩 / 漏识别；本轮不修复模型解码，也不把“片段完成”当作正文完整。自然口播首窗口 `[0, 30000)` 包含多句，末窗口 `[295560, 297530)`，最短 970ms；CPU / CUDA 文字分别共 1816 / 1815 字符，仅记录观测，没有人工完整词边界真值或全文 WER / CER。

因此可交付的是**语音段范围的粗定位**，实际窗口可达约 30 秒，可能跨句或切在词中。不能保证任一句话的独立起止、逐字高亮或准确剪切点。人工改写更不能自动继承原范围，精确同步继续阻塞，当前全文能力保留。

## 视频与提取音频的原点

`scripts/verify_transcript_clock.py` 用现有 FFmpeg 生成 0 / +1500ms 音轨延迟的 Matroska PCM16 样本，以同目录 FFprobe 核对流起点，再执行与生产相同的音频提取参数。FFprobe 仅为本轮验证工具，不新增产品依赖。

两份当前提取的 WAV 均与原 3 秒人声 PCM **逐字节相同**，延迟样本丢掉了前置 1500ms；因此同一 VAD 时间相对视频播放轴会早 1500ms。候选 `-af aresample=async=1:first_pts=0` 在延迟样本补出恰好 24000 个静音采样帧，其后原始 PCM 不变。证据 `.local/wb301/clock/summary.json`。

结论是采用**视频播放轴归零的规范提取音频**后再计算 VAD，复用 FFmpeg，避免加入一套自有词对齐算法。当前只验证上述两种 Matroska 样本；负时间戳、非零容器起点、其他支持容器及实际播放器仍是 WB-302 的前置验证。未确认原点的范围不得伪装成视频时间；当前生产流程暂时仍保持全文能力。

## WB-301 时确定的后续方案

1. 先统一并验证提取音频与视频播放的零点，保留本轮延迟样本作回归；无法确认时不开放片段时间定位。测试通过的滤镜参数是候选实现，尚未接入生产。
2. CPU 使用已有 `--srt` 获取“VAD 窗口 + 对应文本”；CUDA 保留现有 `vad.txt` 与 `transcript_sXXXXXX.txt` 的一一映射，不重新把全文按字数摊到时间轴，也不增加转录模型。
3. Python 生成专用段落文件，建议 artifact schema v1，显式记录 `time_base=media_zero`、`timing_source=fsmn_vad`、`precision=vad_segment`、实际模型 / VAD / pipeline 修订及有序片段。JSONL v3 完成结果可增加可选的受限 artifact 描述，由 Rust 校验固定文件名、字节数、结构和内容；旧引擎无段落时保留全文，不能补造时间。具体字段在 WB-302 实现时同步协议契约。
4. Rust 校验有限非负整数、区间有序、`start < end <= 规范音频时长`、正文和文件版本关联；将段落与新的不可变识别文档、资产文件 ID 和指纹在同一事务保存。使用采样帧处理 EOF，不能假定最后一毫秒完整。失败、取消、冲突保留旧成功文档，沿用现有暂存回收。
5. `transcript_segments` 绑定识别文档及顺序、起止、正文和时间来源。接入 SRT / 规范音频的处理应使用新的 pipeline 修订，不能给旧全文追认时间。人工整理稿可改写或重排，不能直接继承原文时间；引用仍固定旧文档 / 文件。旧资料默认无时间，只有显式重转录才能产生新版本，不按进度或字符数回填。
6. WB-303 / WB-304 以完整语音段作粗定位或取帧范围，显示语音段级范围；不把长语音段伪装成短句，也不声称卡点 / 逐字同步。更细能力继续阻塞，直到独立验证新的时间来源。

WB-302 还需验证取消 / 失败不提交半套段落、无语音 / 缺段处理、CPU `/sil` 省略、版本与文件变化、备份恢复及跨平台，并同步数据库 / 传输格式。以上为后续范围，本轮未实施迁移或对外 API。

## 为什么暂不引入对齐依赖

现有固定 CLI 和 VAD 已提供可追溯语音窗口，满足粗粒度学习定位；本轮不新增 Python ASR 框架或手写对齐算法。针对更细需求已核对成熟方案：

| 方案 | 可用方向 | 当前决定 |
| --- | --- | --- |
| [WhisperX](https://github.com/m-bain/whisperX) | 强制对齐，需要语言相应的声学模型；词表外文本和重叠语音有限制；[BSD-2-Clause](https://github.com/m-bain/whisperX/blob/main/LICENSE) | 需要额外模型 / Python 推理依赖，中英混合与改写正文仍需专项验证；本轮不安装、不将其文档能力算成本项目实测 |
| [faster-whisper](https://github.com/SYSTRAN/faster-whisper) | Whisper 识别和可选 word timestamps，MIT | 会引入另一套识别模型与运行时，不能直接证明现有 Nano 正文已被正确对齐；本轮不切换引擎 |
| 已有 Nano / FSMN-VAD | 沿用现有原生分发及模型，真实语音窗口可复用 | 选为下一步段落范围来源，精度明确标为 `vad_segment` |

以上是有范围的选型结论，不是词级对齐效果验收。没有人工词边界真值集，本轮不给出 ±N 毫秒、WER / CER 或字幕同步达标的数字。

## macOS Apple Silicon 长语音补验（2026-10-29）

本轮在 arm64 / macOS 27.0.1 复用已安装并校验的 CPU 模型、CLI 与 FSMN-VAD；没有安装新工具或模型。证据位于 `.local/macos-workbench-20261029-b39/`，实际命令与原生操作见[第 39 批记录](workbench-knowledge-base-validation.md#第-39-批macos-长语音时间来源复验)。此前短中文、停顿中英、66 秒重复样本、上游 8 秒样本与 11 组媒体时钟证据继续按第 30 / 31 批范围保留，本轮重新运行前三组并增加两个完整长输入。

| 输入与来源 | 时长 / PCM 采样帧 | 独立 VAD / CPU SRT | 最短 / 中位 / 最长窗口 | CPU CLI 耗时 |
| --- | --- | --- | --- | --- |
| 已有本地完整解说视频，非循环 / 拼接；配音是真人或合成尚未核实 | 628.5409375 秒 / 10056655 | 58 / 58 | 540 / 8550 / 30000 ms | 27.971 秒 |
| 《阿Q正传》第一章完整朗读；[LibriVox 官方目录](https://librivox.org/the-true-story-of-ah-q-by-xun-lu/)列出朗读者 Vivienne Wong，目录日期 2016-12-22 | 466.7820625 秒 / 7468513 | 129 / 129 | 750 / 2430 / 30000 ms | 20.089 秒 |

朗读 MP3 由官方目录链接的 Archive.org 下载，7,473,617 字节，SHA-256 `16e30ba525da16e2f46621399d0b4f632f9f93558ab37a62dbd480f9a65ef748`；整章转换为单声道 16kHz PCM16，未裁切、循环或变速，WAV SHA-256 `1117cafcb46a1a5a7c39d324274c14741d427988467348f2d27a6f1acac5a168`。该具名录音补充长真人语音来源，未将来源未核实的解说配音冒充真人。两份输入均只用于本地验证，不加入分发包。

主运行四组的 CPU SRT 起止与独立 VAD 有序窗口一致，均无 `/sil` 省略，输入哈希不变。长解说首末窗口为 `[0, 30000)` / `[621890, 628520)`；真人朗读首末为 `[900, 22270)` / `[461320, 462140)`。真人朗读另经生产 `transcription.recognize` 实跑，129 段起止、逐段文本、合并正文及音频采样帧与独立结果一致；这部分是 Python 生产路径验证，没有原生入库。

长视频通过全新隔离 Tauri profile 原生导入 / 提取原文，实际产生 3938 字、58 段，与独立 CPU SRT 逐条相等；Source / file / document / operation 绑定正确。原生跳转第 41 段 427.390 秒与第 58 段 621.890 秒，播放器在对应位置附近暂停并高亮当前段；重启后原文版本、时间和段落不变，末段仍可跳转。该证据来自实际 ASR 入库，没有用 SQL 夹具构造时间。

时间来源结论不变：**仅语音段级粗定位**。解说 30 秒处词组被拆开、跨多句与末尾不完整表达仍存在；真人朗读也出现专名误识别和跨句长窗口。解说三个烧录字幕短语与所在窗口对应，只是有限画面核对，不是独立听写真值。未做完整人工听审、WER / CER 或词边界真值评估，文本质量不计通过，不修饰识别结果。

原 Windows 297.586 秒自然样本未在当前获准工作区找到，**同输入跨平台比较仍为 NOT_RUN**；本轮用更长的独立完整样本完成同类时间来源复验，不宣称与 Windows 同输入等价。CUDA 为 NOT_RUN，Intel 为 UNAVAILABLE；本轮是开发态原生应用验收，未重新制作正式安装包。
