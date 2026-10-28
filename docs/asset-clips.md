# 片段收藏与范围播放（WB-303）

片段保存原视频引用、开始 / 结束毫秒、标题、收藏理由和可选的已保存文案引用。不生成裁剪视频；没有时间信息的文案也能配合播放器手动标记。Windows 的验证结果见[第二十一批记录](workbench-knowledge-base-validation.md#第二十一批wb-303-片段收藏与范围播放)，macOS Intel / Apple Silicon 继续待验收。

## 存储与绑定

schema v5 新增 `asset_clips`。`file_id` 必须属于当前资产；`source_json` 固定创建时的 canonical path、字节数和纳秒 mtime，沿用已有 Source 契约，不新增内容哈希或媒体副本。播放准备时重新读取 Source；缺失返回 `FILE_MISSING`，重新关联、大小 / mtime / 路径变化返回 `CLIP_SOURCE_CHANGED`。仅保持相同字节数及 mtime 的外部替换不在该指纹的识别保证内。

文案引用保存不可变 `document_id`、`document_version` 和精确 `quote`。引用必须属于同一资产且原文包含该句；沿整理稿来源链找到识别原文时，其 file ID 和 Source 必须与片段所选源一致。新识别版本不改旧引用，重新关联不会把旧片段绑定到新视频。无来源链的人工稿允许手动关联，不由选中文字数推导时间。

标题去首尾空白后为 1～128 个 Unicode 字符；理由最多 16 KiB UTF-8。范围为非负整数毫秒且 `start_ms < end_ms`，保存时必须完整包含于当前播放器报告的一段可播放范围。保存后不自动修改起止或来源；删除只移除该收藏。两种文案导出均把片段的历史源纳入既有媒体保护集合，避免重新关联后覆盖旧视频。

升级 v1～v4 前先执行包含 WAL 的一致性备份，再事务升至 v5；失败保留原版本，可重试。不补造旧文案时间，不支持旧程序直接打开新库。

## Tauri 与 Core

| Tauri 命令 / `libraryApi` | 输入与结果 |
| --- | --- |
| `list_library_clips` / `clips` | `{assetId}` → 片段数组，按 `created_at DESC, id DESC`；含 `missing` / `source_changed`，不携带 Source 路径快照 |
| `create_library_clip` / `createClip` | `{assetId,input:{file_id,title,reason,start_ms,end_ms,playable:{start_ms,end_ms},document_id,quote}}` → 新 ID；可选引用两字段同时为 null 或有效值 |
| `delete_library_clip` / `deleteClip` | `{assetId,clipId}` → 成功；核对归属，不删除视频或文案 |
| `prepare_library_clip` / `clipPlayback` | `{assetId,clipId}` → `{clip,path}`；Core 重新校验源后，由 Tauri 放行该文件的 asset protocol；前端把 path 转为本机媒体 URL |

创建 / 删除成功发 `library_changed {scope:"clips",asset_ids:[assetId]}`；失败不发成功事件。Core 是保存、归属与文件绑定的权威。`playable` 是渲染器当时测得的能力范围，不是媒体时长、授权凭据或全平台可播放承诺；其他渲染器保存的范围仍须在本次实际播放前验证。不扩展现有七项只读 MCP 工具或 Resources。

稳定错误新增 `CLIP_NOT_FOUND`、`CLIP_RANGE_INVALID`、`CLIP_RANGE_UNAVAILABLE`、`CLIP_REASON_TOO_LONG`、`CLIP_SOURCE_CHANGED`、`CLIP_SOURCE_MISMATCH`，复用 `INVALID_INPUT`、`INVALID_CITATION`、`FILE_MISSING`、资产 / 文件 / 文档归属错误。

## 实际播放范围

复用当前 HTML 视频元素及原生 controls，无新播放器依赖。开始时真正 seek 到 `seekable` 首端，检查实际落点，再与有限 `duration`、`seekable` 交集生成可播放范围；监听范围变化刷新编辑器。不会把转录 WAV 长度或登记时长当作播放器上限。

正 PTS MP4 的实际例子：`seekable.start(0)` 报 0，但 seek 0 会落到 5 秒；本轮 Chromium 的 duration 会从 6.692 / 8.804 秒继续变化到 10.2 / 10.9 秒。故可播上限来自当前会话，不能把上轮 8.804 秒写死。范围覆盖失败时暂停并显示当前可播放区间；起点 seek 落点偏差超过 50ms 时拒播。该阈值用于发现定位受限，不是音画或剪切精度保证。

范围播放使用动画帧检查和 `timeupdate` 到点暂停，并回到保存终点；原生控件可暂停 / 恢复，恢复到范围外时重新定位起点。失焦 / 隐藏暂停片段，切文件、离开详情或来源变化释放媒体与回调。这是供学习的范围试听，受播放器帧率、系统调度、解码和浏览器时钟精度影响，不承诺逐帧截断、逐字同步或精确剪辑。部分容器会继续改变 duration；当次拒绝的范围可在重新加载后再检验，不改写已保存区间。

不支持内置解码、不可寻址的完整范围、缺文件、变更源分别保留明确反馈；现有系统播放器入口由用户显式打开，不承诺系统播放器支持同样的范围停止，也不自动转码或修复原媒体。HTML 时钟与 seek 的限制参考 [MDN currentTime](https://developer.mozilla.org/en-US/docs/Web/API/HTMLMediaElement/currentTime) 和 [seeked](https://developer.mozilla.org/en-US/docs/Web/API/HTMLMediaElement/seeked_event)。

## 界面与恢复

播放器区域的“收藏片段”提供起止标记、秒数输入、标题和理由；已保存文案选区提供“引用所选文字收藏片段”。不可用或不匹配的文案来源禁用引用入口，Core 仍执行同一校验。片段列表可播放、查看固定文案版本及删除。片段与整理稿 / 笔记共用离开前的草稿保护；保存失败保留输入。

当前逻辑包 writer **v9**，reader **v1 / v2 / v3 / v4 / v5 / v6 / v7 / v8 / v9**；v9 增加[AI 创建来源](mcp-write-tools.md)，v8 增加[自己的选题与草稿](ideas-and-drafts.md)，v7 增加[研究专题](research-topics.md)，v6 增加[比较笔记](video-comparison.md)，v5 增加[结构化拆解笔记](structured-analysis.md)，v4 保留片段并增加[关键帧资源](asset-keyframes.md)，删除片段会级联删除其图片。v3 增加白名单 `clips`，历史源使用文件名、大小、mtime、`current_file`，不携带源绝对路径。恢复验证归属、版本、原句、范围和来源链指纹；旧版本包混入片段拒绝。随包且指纹一致的媒体重基到新路径，历史来源保持不可用；相同历史源共享安全的内部占位名，支持恢复后再次备份 / 恢复。无媒体包保留收藏、时间、理由及文案引用，缺源继续拒播。

自动段落定位、同步跟随、关键帧及 MCP 片段工具不在 WB-303 范围。

## WB-305 从段落选取

[同步阅读](transcript-sync-reading.md)可选择连续语音段，预填首段开始 / 末段结束和精确文案引用，再进入本页既有片段表单。保存前仍核对完整可播放范围、文案版本和 Source；不因末段超出播放器范围而静默缩短，不跳过中间段落或静音。无时间文案继续支持手动收藏。
