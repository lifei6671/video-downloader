# 关键帧提取与资源管理（WB-304）

在资产详情中从播放器当前位置取帧，或从已保存片段均匀提取 3 / 5 / 9 帧。这里的“关键帧”是学习用的画面资源，不要求输入视频的 I 帧，也不保证与播放器截图完全相同。Windows 验证见[第二十二批记录](workbench-knowledge-base-validation.md#第二十二批wb-304-关键帧提取与资源管理)；macOS Intel / Apple Silicon 继续待验收。

## 时间和采样

沿用媒体呈现 PTS 零点，不减去容器 / 首帧起点。指定位置取第一张 PTS 大于等于请求时间的可解码视频帧；保存整数 `pts`、`time_base_num/den`，`actual_ms` 为换算后的四舍五入毫秒。请求时间与实际时间分别显示。例如 10 fps 视频请求 350ms，返回实际 400ms；首帧为 5 秒的视频请求 5350ms 返回 5400ms。首帧前、尾部无画面、片段结束后才有画面均明确失败，不把其他位置冒充请求结果。

片段区间为 `[start_ms,end_ms)`，第 i 个请求点为 `start + floor((end-start)*(2*i+1)/(2*count))`。Core 接受 1～9 帧，要求区间至少 count 毫秒；界面提供 3 / 5 / 9。实际返回画面仍须位于原区间内。不同请求可能落在同一帧，不伪造不同时间。

复用 FFmpeg 的 `copyts`、绝对输入 seek、`noaccurate_seek` 和 select / showinfo，先探测第一张非负 PTS 帧，再提取目标位置；`noaccurate_seek` 避免正起点 MP4 二次偏移。取非封面视频流，保持显示宽高比（包括非方形像素），输出方形像素 PNG，长边最多 1280px，单张最多 8 MiB。灰度低于 24 的像素占比达到 98% 时标记黑帧并保留原图。提取不裁剪或改写原视频。

## 存储与生命周期

schema v6 的 `asset_keyframes` 保存 PNG BLOB、不可变 Source 指纹和元数据：资产 / 文件 / 可选片段 ID、批次 ID、采样区间、序号 / 总数、请求时间、实际 PTS、尺寸、黑帧标记及创建时间。列表只返回摘要，图片按资产与关键帧 ID 单独读取；PNG 经完整解码及结束块校验。复用已锁定的 [image-png](https://github.com/image-rs/image-png)，MIT / Apache-2.0，许可随安装包分发。

Source 变化或缺失时已有图片继续可读，并显示来源状态；禁止用新源替代旧图来源。删除图片只删除图片，删除片段级联删除其图片，删除资产沿现有归属级联处理。无文件系统缓存目录或永久外部图片路径。

全应用同时运行一项提取，使用现有 Core worker 与 PythonBridge 进程树管理。取消 / 退出先回收引擎及 FFmpeg，再清理本批暂存目录；读取、校验和暂存清理完成后复查 Source 并一次事务提交整批。失败、取消或事务失败不会留下半批图片，也不影响旧图。单次解码超时 20 秒，协议操作上限 240 秒。工作台显示真实运行状态，取消完成不能早于进程回收。

引擎 **0.17.0** 与 Rust 精确握手，JSONL 协议仍 v3，新增 `extract_keyframes` / `keyframe_progress` / `keyframes_extracted`；需重建 sidecar，不改变 ASR 模型安装契约。

## 界面与备份

图库显示真实图片、实际时间、请求时间、文件版本、片段采样范围与黑帧 / 来源状态。点击预览，Escape 关闭；图片读取失败可重读，删除失败保留图片。初始显示 12 张，按需再加载 12 张；离开详情释放 Blob URL，后台任务仍由 Core 管理。配色与边框沿用 DESIGN.md。

当前逻辑包 writer **v9**、reader **v1 / v2 / v3 / v4 / v5 / v6 / v7 / v8 / v9**，v9 增加[AI 创建来源](mcp-write-tools.md)，v8 增加[自己的选题与草稿](ideas-and-drafts.md)，v7 增加[研究专题](research-topics.md)，v6 增加[比较笔记](video-comparison.md)，v5 增加[结构化拆解](structured-analysis.md)；v4 首次新增 `keyframes` 及带大小 / SHA-256 的图片 payload；不包含视频时也保留全部图片。恢复完整校验归属、采样与时间、PNG、片段和 Source 关系，失败不发布恢复目录。文案、片段和图片共用历史 Source 重定位，恢复后再次备份仍保留同一来源身份。旧 reader 拒绝新版本，旧包不能混入图片数据。当前 schema v11 保留 v6 图片表，schema v1～v10 升级前保留一致性备份，迁移失败事务回滚，旧资料仍可恢复。

本批没有实现逐帧剪辑、文案同步跟随、画面分析或 MCP 图片读取；后者仍由 WB-312 负责。截图只证明采样位置的画面，不代表已看完连续动作或听过声音。

FFmpeg 行为依据：[命令文档](https://ffmpeg.org/ffmpeg.html)、[showinfo](https://ffmpeg.org/ffmpeg-filters.html#showinfo)。
