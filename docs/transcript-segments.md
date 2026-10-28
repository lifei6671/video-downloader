# 可追溯的语音段时间

对应 [WB-302](workbench-knowledge-base-tasks.md)。已实现媒体时间原点处理、转录时间产物、Rust 校验、schema v4 存储及备份格式 v2，完成 Windows 验证。macOS Intel / Apple Silicon 继续待验收；没有新增同步播放界面、片段操作、逐字对齐或 MCP 时间工具。

## 时间坐标与精度

`time_base=media_zero` 指媒体呈现时间戳 PTS 的零点。它不等于第一包、第一帧或第一句语音的起点。提取复用 FFmpeg：

```text
-copyts ... -map 0:a:0 -vn -ac 1 -ar 16000
-af aresample=async=1:first_pts=0 -c:a pcm_s16le -f wav
```

保留呈现时间，正起点补前置静音，零点前的音频裁掉；不使用 `-start_at_zero` 减去容器起点。依据 [FFmpeg copyts 契约](https://ffmpeg.org/ffmpeg.html#Advanced-options)及[重采样参数](https://ffmpeg.org/ffmpeg-resampler.html#Resampler-Options)实现，不另写时间换算器、不新增 FFprobe 运行依赖。

WB-301 只验证了零起点和音轨延迟；本轮发现减去容器起点会与实际播放器不符：测试 MP4 的容器起点为 5 秒，Edge 的低位 seek 会落到 5 秒。最终保留 PTS，原始音轨从 6.5 秒开始时，规范 PCM 保留 6.5 秒静音，而非减成 1.5 秒。矩阵还覆盖负音轨、MOV、AAC、Opus 和 VFR。

`timing_source=fsmn_vad`、`precision=vad_segment`，仍是 [WB-301 验证的语音段范围](transcript-timing-validation.md)，可达约 30 秒，可能跨句或切在词中。10ms 是 VAD 计算网格；有损编码的起音观测和浏览器 seek 都不代表逐句 / 逐字或音画同步精度。

## 内部产物与进程协议

WB-302 引擎为 **0.16.0**（当前 WB-304 为 **0.17.0**），JSON Lines 协议仍 v3，Rust 握手精确匹配引擎版本；旧 sidecar 须重建，不能静默保留旧提取流程。CPU 从固定 CLI `--srt` 解析窗口及正文；CUDA 保留原 VAD 裁剪 / 逐片段文本映射。全文统一为段落正文以换行连接；`/sil` 不充当正文，正 VAD 片段的空 / 缺失 / 非 UTF-8 输出仍失败。

生产私有暂存新增 `transcript-segments.json`，artifact schema v1：

```json
{
  "schema_version": 1,
  "time_base": "media_zero",
  "timing_source": "fsmn_vad",
  "precision": "vad_segment",
  "sample_rate": 16000,
  "frames": 72000,
  "vad_revision": "6840bae4c5c92ee8c04faaf4db23dd0105098d7f",
  "segments": [{"start_ms": 1920, "end_ms": 4480, "text": "开放时间早上九点。"}]
}
```

`transcribed` 仍只返回全文文件描述和状态，增加必需的 `timing: {filename: "transcript-segments.json", bytes: N}`，不把正文或整个时间文件放进 stdout / 进度事件。Rust 只读取该固定文件，检查普通文件、字节数和 16 MiB 上限，拒绝未知字段、错版本 / 原点 / 来源 / 精度、布尔或浮点冒充整数、逆序 / 重叠 / 空区间、超出实际音频采样边界、空正文和全文不一致。EOF 使用实际采样帧数，毫秒末端不向上虚增。

Core 在校验后绑定自己的 `operation_id`，不从进度计数生成时间。落库和读取的 `timing` 带该 ID；它也是识别幂等键的一部分，确保同秒的两个成功操作仍各自生成不可变版本，重复保存同一个结果不复制版本。

模型及 VAD 权重不变，不重新下载同一模型。原安装 manifest 的就绪修订保持兼容；新结果的处理修订分别为 `fun-asr-nano-cpu-fbbfe018-media-zero-v2` / `fun-asr-nano-audiocpp-50749561-media-zero-v2`，不能给旧结果追认本轮时间语义。

## 存储、版本与恢复

- schema v4 新增 `transcript_timings`（文档、采样帧数、VAD 修订、操作 ID）及 `transcript_segments`（文档、顺序、起止和正文），级联绑定不可变 `asset_documents`。模型、后端、pipeline、文件 ID 及原文件 `Source` 快照继续随识别文档保存；Source 指纹沿用规范路径、大小、纳秒 mtime，不宣称内容哈希身份。
- 文档、完整段落和操作快照使用既有 Core 事务一起提交。写段落或操作状态失败全部回滚，保留旧成功结果；启动恢复仍将未完成操作标为中断，不补造段落。取消、无语音、非法产物或源文件改变也不能替换旧正文 / 时间。
- 旧库 1 / 2 / 3 先用 SQLite 一致性备份保存 WAL 已提交内容，再事务升级到 4；失败保留原版本，可修复后重试。旧文案 `timing=null`，不会从字符数或进度回填。`None` 不进入旧识别序列化键，旧结果的幂等保存保持兼容。
- 重识别创建新版本，旧文案和时间保持原绑定。修改 / 替换 / 重新关联文件后，旧范围仍可阅读，`source_changed=true`；不能据此直接操作新文件。人工整理稿不自动继承来源文档的时间。
- `get_library_document` 增加可选的 `timing`，与既有 `summary.file_id`、文档 ID / version、`source` 和 `source_changed` 一起读取。原文 / 整理稿和完整结果读取兼容旧数据；列表摘要与进度仍不携带完整段落。未扩展 MCP 读取范围。
- WB-302 引入 v2 时间字段；当前 WB-310 的逻辑备份 writer 为 **v9**，reader 支持 **v1 / v2 / v3 / v4 / v5 / v6 / v7 / v8 / v9**，增加[选题与草稿](ideas-and-drafts.md)，保留[研究专题](research-topics.md)，保留[比较笔记](video-comparison.md)，保留[结构化拆解](structured-analysis.md)，增加[关键帧图片](asset-keyframes.md)，同时保留[片段引用](asset-clips.md)；旧 reader 应明确拒绝新版本。v2 段落随文档保留；v1 没有时间，不能伪造。恢复重定位 Source 后重算识别键，避免下次保存把旧文档重复成新版本。含 / 不含媒体的恢复均保留历史范围；未恢复或不匹配的媒体继续标变化。

## Windows 验证与边界

| 检查 | 实际结果 |
| --- | --- |
| 生产提取时钟 | 11 组通过；Matroska / MOV PCM、MP4 AAC、WebM Opus、VFR、负起点、+1.5s 音轨及 +5s 容器起点。6 组 PCM 逐字节一致；AAC 起音误差为 0，WebM 延迟样本 16 帧即 1ms，其余 Opus 为 0，均在测试 codec frame 容忍内。这不是词级准确率 |
| Edge 解码及 seek | 5 组通过，含非零 MP4 的 5.2 / 6.6 / 7 秒坐标；没有浏览器端音频起音采样，不作为音画同步精度或 Tauri GUI 验收 |
| 真实 Core / 引擎 | 源码 CPU、源码 CUDA、PyInstaller 打包 CPU 均通过：真实导入、转换 / ASR、两份不可变识别版本、时间入库、旧版不变、重开绑定及原文件不变；延迟样本为 72000 帧，范围 `[1920,4480)` |
| 领域 / 恢复回归 | 验证同秒不同操作、版本 / 文件关联、整理稿无时间、源变化、段落插入中途失败回滚 / 重试、v3 升级与失败恢复、v1 兼容和 v2 含 / 不含媒体恢复 |
| 平台与 UI | macOS 两种架构 UNAVAILABLE；没有新增 UI，段落跳转 / 片段播放与实际 Tauri 播放边界仍由后续 WB-303 等任务验证 |

异常非零起点 MP4 在 Edge 中报告的 duration 为 8.804s，与原始音轨末端 PTS 不完全相同；本轮只验证了列出的 seek 坐标，不承诺该文件的全部范围可在内置播放器播放。后续播放入口必须使用实际播放器可寻址范围并处理不支持的范围，不直接用规范音频长度当作播放器 duration。本轮不重封装或修复用户媒体。

完整计数、失败试跑、构建与独立审查见[第二十批实施记录](workbench-knowledge-base-validation.md#第二十批wb-302-媒体时间原点与可追溯段落存储)，复验入口见[开发文档](development.md#wb-302-段落存储与媒体时钟验证)。本轮没有发布、提交或推送。

## WB-305 同步消费

[同步阅读器](transcript-sync-reading.md)直接消费当前不可变识别文案的 timing，并核对所选文件与 Source；间隙不高亮，旧版本保留原时间。段落跳转不把语音段精度提升为逐句 / 逐字，关闭自动滚动仍可自由阅读。连续段落可预填片段范围与固定原文引用；整理稿不沿用识别时间。当前 UI 实现和 Windows 验证见[第二十三批](workbench-knowledge-base-validation.md#第二十三批wb-305-视频与文案同步阅读)，macOS 继续待办。
