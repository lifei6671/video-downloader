# Fun-ASR-Nano Windows CUDA 加速

2026-10-23 用户批准 **audio.cpp CUDA + 独立 FSMN-VAD**。协议 v3、引擎 **0.12.0**；重建 native helper、PyInstaller sidecar 和桌面程序。macOS Intel / Apple Silicon 保留 CPU。

## 版本选择与安装

系统设置 → 运行依赖只有一张语音模型卡。Windows x64 可选 CPU / CUDA，选择后点击下载或“安装并切换版本”；首次下载弹窗提供相同选择。选择框本身不探测显卡、不下载、不改变当前版本。安装和真实推理成功后才保存选择。

- CPU 保留原 `llama-funasr-cli`、F16 编码器、Q8_0 解码器和 FSMN-VAD，模型约 1.2 GiB。
- CUDA 使用 audio.cpp 单文件 Q8_0 GGUF + 相同 FSMN-VAD，模型约 1.0 GiB；首次还需下载约 1.0 GiB 的程序与 CUDA DLL。GGUF 布局不同，不能直接复用 CPU 编码器/解码器；已下载 VAD 经 SHA-256 校验后复用。
- 目录分别为 `runtime/fun-asr-nano/`、`runtime/fun-asr-nano-cuda/`，选择保存于 `runtime/fun-asr-nano-backend.json`。没有选择文件时仍为 CPU。旧 SenseVoice 目录与选择文件保留但不参与 Nano。
- 当前版本点击“重新下载模型”是修复。切换至完整已安装版本时重新校验权重、真实离线推理，不重复下载。模型、程序和选择在验证通过后提交，提交后检测失败逆序回滚；失败或取消保留原版本。Rust 在启动安装进程前保存原选择与目标存在性；即使 sidecar 在目录移动或选择写入后被终止，也会在回收进程后从暂存备份恢复原目录与选择，再清理暂存。恢复自身失败则保留备份并明确报错。
- 普通启动与重新检测仅查 manifest、文件和大小。用户请求安装 CUDA 或开始 CUDA 转录时才查询 NVIDIA Driver API；不兼容先报错，不下载，不静默回退 CPU。
- 不安装驱动、CUDA Toolkit、PyTorch 或 Python ASR 环境，不启动监听服务，不改全局 PATH。CUDA DLL 仅加入本次子进程环境。

## 独立 VAD 处理

1. FFmpeg 仅读取原视频，生成 16 kHz 单声道 PCM16 WAV。
2. 内置 `llama-funasr-vad` 加载 `fsmn-vad.gguf`，以 `VAD_MAXSEG=30000` 输出毫秒区间；不加载 ASR 编码器或 LLM。该目标复用固定 FunASR 源码，无自研 VAD 算法。
3. 校验区间有序、不重叠、边界合法，按原始 PCM 样本裁剪 WAV；没有语音返回 `no_speech`。
4. audio.cpp 使用 `--backend cuda --device 0 --batch-audio-dir ... --audio-chunk-mode none`，只加载一次模型，按顺序识别全部片段，关闭内部再次切块；子进程仅看到选定 GPU UUID。
5. 每个正 VAD 区间必须有非空 UTF-8 输出；缺失、空输出、超限或进程失败均明确报错，不把漏段拼成成功结果。这不保证每段内部没有错字或漏词。
6. 合并后由 Rust 校验原视频身份并保存。CUDA 结果绑定 revision `ce72677f84900f0dc57f498ace253bfb3c9155b6`、pipeline `fun-asr-nano-audiocpp-50749561-fsmn-v1`、backend `cuda`。旧文案保持原模型/后端，复制和 TXT 导出不变。

转换最多 1800 秒，分段和识别共用 `max(600, 3×音频秒数+120)` 秒预算、每 5 秒心跳。安装/转录互斥；取消、超时、退出由既有 Windows Job / 进程组回收全部辅助进程，再清理暂存。Windows 显式终止 Job 并最多等待 5 秒至全部进程退出；已校验的自有暂存目录仅对共享/锁定错误 32/33 重试最多 310ms，持续失败保留登记并报错，不用取消状态掩盖清理错误。PyInstaller 启动 audio.cpp 时临时清除自身 DLL 搜索目录，启动后恢复。

## 固定来源与兼容边界

采用本机对照实验已验证的 [audio.cpp v0.9.2-drama3-base-small](https://github.com/0xShug0/audio.cpp/releases/tag/v0.9.2-drama3-base-small)，源码 `50749561`，Windows CUDA 12.4。仅解压固定 CLI、DLL 和 LICENSE 白名单，不安装服务器、模型管理器或其他工具。

| 资产 | bytes | SHA-256 |
| --- | ---: | --- |
| `audio-v0.9.2-drama3-base-small-bin-windows-x64-cuda12.4.zip` | 464268606 | `48852da40790c40f3285bca6c53ff7829b3963080f0e95b6742f868fc1d7c757` |
| `audio-v0.9.2-drama3-base-small-cudart-windows-x64-cuda12.4.zip` | 607273675 | `4b1186a525c9ac70a4fbd031b47e0dfca2ebaa36cfcb760d5be1a76eb2b12da5` |
| `fun-asr-nano-2512-q8_0.gguf` | 1045334432 | `4d727357574b079b7f43336b2930f39da086ca02f5d8d50872090b4c1c3d5e0a` |

模型来源 [FunAudioLLM/Fun-ASR-Nano-2512-GGUF](https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-2512-GGUF/tree/ce72677f84900f0dc57f498ace253bfb3c9155b6)。VAD 来源和 SHA 见 [Nano 集成说明](fun-asr-nano.md)。保留 audio.cpp Apache-2.0、模型 Apache-2.0、已有 FunASR/llama.cpp/miniaudio 许可和 [CUDA 12.4.1 EULA](https://docs.nvidia.com/cuda/archive/12.4.1/eula/index.html) 本地副本。EULA 文件 `vendor/nvidia-cuda-12.4-EULA.html`，85051 bytes，SHA-256 `c0e213c4c53fbc9b29dc3e8cea0fb06b0498a28eca01858d3611d975dd70f585`。

固定包要求 Windows x64、Driver API >=12040、计算能力 7.5 / 8.0 / 8.6 / 8.9 / 9.0 之一，安装仍需真实 CUDA 冒烟。范围依据固定上游发布脚本的架构配置；仅 RTX 4070 SUPER（8.9）已在本机实测。CUDA 12.4 包不承诺 Blackwell / RTX 50。

安装沿用受限 HTTPS、系统网络/代理、大小和 SHA-256 校验、串行安装与回滚；不携带平台 Cookie/代理，不接受任意 URL。

## 验证入口

```powershell
.venv/Scripts/python.exe scripts/verify_transcription.py --variant cuda --download-models --runtime-directory .local/nano-cuda-verification/runtime --engine python-engine/dist/video-engine/video-engine.exe
.venv/Scripts/python.exe scripts/verify_transcription.py --variant cuda --runtime-directory .local/nano-cuda-verification/runtime --source 'F:/Videos/example.mp4'
```

`--asset-cache` 可读取已有固定官方资产，仍经生产安装器完整校验和真实推理，不冒充本轮完整网络下载。`--runtime-directory` 仅允许仓库 `.local` 内隔离目录。实际本轮验收和平台缺口见 [验证记录](nano-cuda-validation.md)。
