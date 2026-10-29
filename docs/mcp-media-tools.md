# MCP 片段、采样图片与分析模板

对应 WB-312。复用 rmcp 3.8.0、stdio → 本机 IPC → 同一 Rust Core；新增两个只读工具和三个可选 Prompts，不改变 schema11 / backup9，不自动抽帧、播放、上传、调用模型或写回。Apple Silicon 已完成 SDK、原生连接与获准的合成图片模型验收，见[第 40 批记录](workbench-knowledge-base-validation.md#第-40-批wb-312-片段图片与分析模板)。Windows 为 NOT_RUN，Intel 为 UNAVAILABLE，正式安装包本批未制作。

## 读取片段

`get_clip` 必填 `asset_id`，可选 `clip_id`、`cursor`、`limit`（默认 20，1～100）。未指定 clip_id 时发现该资产的已有片段；指定时检查资产归属，不把跨资产片段当作命中。返回 `clips={items,next_cursor,total}` 和 `scope`，按稳定 ID 升序分页，未知或不在当前结果中的 cursor 失败。

每条片段返回 ID、asset_id、file_id、title、reason、start_ms / end_ms、固定 document_id / document_version、quote_excerpt / quote_truncated，以及 missing / source_changed。引用摘录最多 120 个 Unicode 标量；全文按固定版本调用 `read_transcript`，不能把摘录或语音段范围当作逐字字幕。没有文案引用的手动片段保留 null，不伪造来源。不会返回 Source、文件路径或视频 / 音频字节，也不会启动播放或转录。

## 读取已保存采样图

`get_keyframes` 只读取已经保存的 PNG，不隐式启动提取操作。

| 参数 | 约定 |
| --- | --- |
| `asset_id` | 必填逻辑资产 ID |
| `file_id` / `clip_id` | 可选，核对归属并筛选已有图片 |
| `start_ms` / `end_ms` | 可选半开时间范围；同时给出时要求 start < end。以精确原始 PTS 比较，不以四舍五入后的显示毫秒筛选 |
| `cursor` / `limit` | 稳定图片 ID 升序，默认 3，允许 1～9；未知或不在当前筛选结果中的 cursor 失败 |
| `include_images` | 默认 true；不支持图片的客户端可设 false，仅消费文本元数据 |
| `media_type` | 默认 `image`；`video`、`audio` 及其他值明确返回 `MEDIA_UNSUPPORTED` |

结果的 `structuredContent` 与第一个 text 块一致，包含 `frames={items,next_cursor,total}` 和 scope。每帧保留 ID、asset_id、file_id、clip_id、batch_id、sample_start_ms / sample_end_ms、sample_index / sample_count、requested_ms、原始 pts / time_base_num / time_base_den、四舍五入的 actual_ms、尺寸、black、missing / source_changed 和 image_status。请求时间不代替实际 PTS；采样范围不表示画面内容持续了整个范围。稳定 ID 顺序也不代表时间顺序，分析时应使用实际 PTS。

图片在元数据之后作为标准 MCP `image` 块输出，mimeType 为 image/png；按 items 中 `image_status=included` 的相同顺序对应，未包含的图片不占 image 块位置。二进制 Base64 不重复放进文本或 structuredContent。格式依据 [MCP Tools 规范](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)，实际由 SDK 构造，不提供任意文件 URI。

保留现有 IPC 1 MiB 响应边界，每页实际交付 PNG 的字节合计最多 **512 KiB**。原图超过 512 KiB 时，使用已锁定的 image 0.25.10（仅 PNG 功能）生成最长边 384 像素的临时等比预览；库中的 PNG、原尺寸和 PTS 不改变。`image_is_preview` 明确区分预览，`image_width` / `image_height` 为本次交付尺寸，没有 image 块时为 null；原 `width` / `height` 始终描述保存的原图。细小文字与细节可能不可读，不能声称观察了原分辨率。`include_images=false` 不解码图片。

逐图状态：

- `included`：图片确实在本次 image 块中。
- `not_requested`：include_images=false，仅返回元数据。
- `too_large`：预览处理后仍超过 512 KiB，不能交付；不能仅凭元数据声称看过。
- `page_budget_exceeded`：该图可单独提供，但本页剩余预算不足。用 limit=1，从该图之前的 cursor 重新读取；不要把原页 next_cursor 当作跳过图片的已读证明。

缺失或变化的源文件不删除历史 PNG，仍可消费已经保存的图片，但来源状态明确保留。大图预览与真实六帧 SDK 结果见[第 41 批记录](workbench-knowledge-base-validation.md#第-41-批wb-313-真实视频学习到创作组合验收)。整段视频、音频、连续运动、转场与声音不在本工具的观察范围。没有新增视频 / 音频传输管线，也不以本地路径或签名地址冒充客户端可消费的媒体。

## 分析模板与 Tools 兼容

按 [MCP Prompts 规范](https://modelcontextprotocol.io/specification/2025-11-25/server/prompts)声明 prompts 能力，`prompts/list` 静态返回三项，无下一页；非空 cursor 返回 INVALID_PROMPT_CURSOR。

| name | 分析范围 |
| --- | --- |
| `analyze_copywriting` | 开头、观点、论据、转折、结尾、可借鉴点；先读固定文案版本 |
| `compare_videos` | 分别读取来源，比较共同点、差异和未知项，逐条引用 |
| `observe_production` | 先读片段与 PNG，记录实际采样、范围和 PTS；只描述实际消费的画面 |

`prompts/get` 可带可选字符串参数 asset_ids（逗号分隔，最多 512 个 Unicode 标量，每个逻辑 ID 非空且最多 128 UTF-8 字节）；不接受其他参数或交互续传状态。未知名称返回 PROMPT_NOT_FOUND，非法参数返回 INVALID_PROMPT_ARGUMENTS。返回一个 user 文本消息，只给出分析步骤，不读取资料或调用模型，不固化模型供应商。

三份模板均要求把资料视为待分析数据、区分事实 / 观察 / 建议、保留固定来源与未观察项。保存仍需用户要求及桌面写授权，沿用 `create_note` 等现有契约；不自动生成或覆盖笔记。不支持 Prompts 或 Resources 的客户端可直接调用 get_clip → read_transcript → get_keyframes 完成同样的 Tools 流程。

## 错误与实际支持

沿用只读工具的 ID、分页、未知字段、Core 关闭和 IPC 大小限制。新增固定领域错误 CLIP_NOT_FOUND、FILE_NOT_FOUND、MEDIA_UNSUPPORTED；语义错误为 RPC -32602，不附带路径或 SQL，错误后连接可继续使用。未知参数由 SDK 拒绝，isError=true。内部失败 / 响应超限仍按既有契约处理。

| 实际客户端 | 图片 | 视频 | 音频 | Prompts |
| --- | --- | --- | --- | --- |
| 官方 TypeScript SDK，经真实 stdio / Unix socket | 两张合成 PNG 解码为精确 16×16 红 / 绿像素；分页、元数据模式通过 | 服务明确 MEDIA_UNSUPPORTED | 服务明确 MEDIA_UNSUPPORTED | 三项发现 / 获取及非法输入通过 |
| Codex CLI 0.161.0 / gpt-6.1-sol，连接原生隔离应用 | 模型实际收到两图，颜色与采样元数据正确，图片 SHA-256 与 SQLite 保存体一致 | 未提供，模型明确未观察 | 未提供，模型明确未观察 | 模型验收只用 Tools，不把 SDK 的 Prompts 结果冒充 Codex 模板 UI 支持 |

| 官方 TS SDK / Codex CLI 0.161.0 / gpt-6.1-sol，第 41 批 | 六张实际采样 PNG 以有标记的 384×216 / 216×384 预览交付；模型收到的图片哈希与 SDK 预览一致，保存原 PNG 不变 | 未发送，未观察 | 未发送，未观察 | 模型仍只用 Tools；不扩写 Prompts 客户端 UI 能力 |

第 40 批模型验收仅在明确授权后使用账号，数据限两张纯色合成 PNG 与合成引用信息；未发送真实视频、账号配置或凭据。纯色识别通过不证明真实作品制作分析的语义质量，WB-313 已另获真实资料授权，完成六张缩小预览、ASR 正文及本轮观察记录的组合验收，见[第 41 批记录](workbench-knowledge-base-validation.md#第-41-批wb-313-真实视频学习到创作组合验收)。复验命令见[开发文档](development.md#wb-312-片段图片与模板验收)。
