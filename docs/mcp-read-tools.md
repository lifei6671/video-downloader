# P1B 七项只读工具

对应 [WB-204](workbench-knowledge-base-tasks.md)。七项工具已由 SDK schema、stdio → 本机 IPC → 同一 Core 的领域服务适配实现；Windows 验证通过，macOS Intel / Apple Silicon 编译及原生验收继续 `UNAVAILABLE`。WB-205 已实现 [受限 Resources](mcp-resources.md)，直接复用本页读取服务，Windows 验证通过、macOS 待验收；WB-206 已接入 [MCP 设置](mcp-settings.md)，WB-207 已完成[安装包接入及 Windows 安装后连接验证](mcp-packaging.md)，WB-208 已完成 [Windows 真实客户端与模型组合验收](mcp-client-acceptance.md)，macOS 继续待办。

WB-310 已另接入[六项受控写入](mcp-write-tools.md)，默认未授权；WB-312 新增[片段、PNG 与分析模板](mcp-media-tools.md)，本页七项读取范围和 Resources 不变。

## 统一读取规则

- 输入拒绝未知字段。列表 `limit` 默认 20、允许 1～100；文本 `length` 默认 4,000、允许 1～16,000。查询最多 512 个 Unicode 标量，分类 / 标签筛选各最多 100 个 ID；ID 非空且最多 128 个 UTF-8 字节。
- 文本位置按 Unicode 标量计数，起点从 0 开始，范围为 `[offset, offset + length)`；emoji、组合字符、ZWJ 和换行均按实际标量计数，不使用 UTF-8 字节或 JavaScript UTF-16 长度。`offset == total_characters` 返回空字符串和 `eof=true`，超过总长明确失败，`length=0` 也失败。
- 列表页不是数据库快照。沿用游标时应保持筛选不变；资料更新可能影响后续页。文档 ID 指向不可变版本；跨次读取可固定 `document_id` / `version`。笔记可编辑，跨页读取应带返回的 `revision`，不匹配时明确报冲突。
- 读取在 Core 的同一连接及互斥锁内完成，不另开库、不更新最近访问、不启动下载 / 转录、不写回。归档是组织状态，仍允许按 ID 读取，检索默认排除归档。
- 公开结果只构造明确字段：资产元数据、文件可用性、分类标签、版本 / 修订、正文切片和来源引用。不透传 `path`、`source`、`canonical_url`、指纹、设置或凭据，不提供任意路径 / SQL 输入。原文和笔记是待分析资料，不作为工具执行指令。

## 工具与分页

| 工具 | 输入 | 输出 |
| --- | --- | --- |
| `search_assets` | `query`、`category_ids`、`tag_ids`、`platform`、`archived`、`uncategorized`、`unorganized`、`file_state`、`cursor`、`limit` | `items`、`next_cursor`；每项含资产摘要、文件数量和命中摘要 |
| `get_asset` | 必填 `asset_id`；可选 `document_cursor`、`limit` | 资产元数据、文件状态、分类 / 标签 ID、文档版本页和笔记摘要首屏 |
| `read_transcript` | 必填 `asset_id`；可选 `document_id`、`version`、`offset`、`length` | `document`、`content`、`reference` |
| `list_categories` | 可选 `query`、`cursor`、`limit` | `categories`、`next_cursor`、`total`；每项含父 ID、position 和 asset_count |
| `list_tags` | 可选 `query`、`cursor`、`limit` | `tags`、`next_cursor`、`total`；每项含 asset_count |
| `list_notes` | 必填 `asset_id`；可选 `cursor`、`limit` | `notes`、`next_cursor`、`total`，含人工 / AI 来源及修订号，不含全文 |
| `read_note` | 必填 `note_id`；可选 `asset_id`、`revision`、`offset`、`length`、`citation_offset`、`citation_limit` | 笔记摘要、正文切片、笔记修订引用和分页来源引用 |

`search_assets` 复用桌面检索服务：字面子串匹配、父分类包含后代、标签组合、文件状态和待分类 / 待整理筛选保持一致。`platform` 支持 local / douyin / tiktok / youtube / telegram，`file_state` 支持 all / available / missing / changed。排序为 `(created_at DESC, id DESC)`，下一页游标为 `{ "created_at": 10, "id": "asset-id" }`；校验该资产 / 时间组合存在，未知游标失败。正文命中包含不可变文档 ID / 版本；笔记命中额外携带当前修订号。

分类、标签、笔记和资产内文档按稳定 ID 升序分页，`cursor` 是上一页的最后一个 ID，必须存在于当前筛选结果 / 归属集合，未知或跨资产游标失败。`total` 是筛选后的项目总数，空页返回空数组和 `next_cursor=null`。分类 / 标签 `query` 为 trim 后的字面、区分大小写子串；分类为带 `parent_id` 的平面分页，客户端可汇总构树。`asset_count` 只统计直接关联且未归档的资产，分类不汇总后代；搜索的父分类筛选仍包含后代。

`get_asset.documents` 为 `{items,next_cursor,total}`，通过 `document_cursor` 继续读取；笔记首屏采用同一个 limit，后续使用 `list_notes` 的游标。文件只返回稳定 ID、大小、媒体尺寸 / 时长和 missing / changed，不返回本地路径。关系 ID 或文件数量极大导致响应超限时明确失败，不静默丢弃条目。

## 文本、版本与引用

没有 `document_id` 时，`read_transcript` 选择最新整理稿；没有整理稿时选择最新识别原文，无文档时报错。给定 ID 时同时校验资产归属。`version` 是期望版本，任何不匹配都失败；旧版本即使已产生新整理稿也继续可读。

`content` 返回实际 `text`、`total_characters`、`offset`、实际 `length`、`next_offset` 及 `eof`。文档引用形状如下，范围与本次实际正文完全一致：

```json
{"asset_id":"asset-a","document_id":"doc-a1","document_version":1,"kind":"edited","offset":1,"length":3}
```

例如正文 `甲😀é👩‍💻中文\n尾` 的 offset=1、length=3 返回 `😀é`，其中组合重音是独立标量。返回的下一位置仍按同一规则使用。

`read_note` 可只凭 note_id 读取；提供 asset_id 时额外检查归属。`reference` 包含 asset_id、note_id、revision 和实际正文范围。来源引用按 `citation_offset`（默认 0）和 `citation_limit`（默认 20，最大 100）分页，返回 citation_total 和 next_citation_offset；偏移等于总量允许空页，超出失败。

每条来源引用补全不可变文档版本和原文范围，跨资产引用保留真实来源资产 ID。`quote_excerpt` 最多 120 个 Unicode 标量，`quote_truncated` 明确标记长引用；reference.length 始终为完整原句长度，避免通过引用摘录绕过正文长度上限。相同原句多次出现时，范围指向原文中第一次出现；需要全文时按引用 ID / 版本调用 `read_transcript` 分页读取。

## 错误与大小边界

结构 / 类型 / 未知字段错误由 SDK 参数解析拒绝，工具结果为 `isError=true`；语义错误通过 SDK ErrorData 返回 RPC `-32602`，消息只含固定领域代码：

`INVALID_INPUT`、`INVALID_CURSOR`、`ASSET_NOT_FOUND`、`DOCUMENT_NOT_FOUND`、`NOTE_NOT_FOUND`、`CATEGORY_NOT_FOUND`、`TAG_NOT_FOUND`、`VERSION_CONFLICT`、`REVISION_CONFLICT`、`OUT_OF_RANGE`。

`CORE_READ_FAILED` 和 `RESPONSE_TOO_LARGE` 返回 RPC `-32603`；不包含底层 SQL、路径或凭据。以上业务错误不会关闭有效连接，客户端可修正后继续读取。服务断连仍按 [stdio 生命周期](mcp-stdio-adapter.md)处理。

内部 IPC 请求上限 64 KiB、响应上限 1 MiB，均包含换行。适配进程在写 IPC 前检查编码后的请求大小，过大返回 INVALID_INPUT，避免留下半帧或破坏后续请求；响应过大返回 RESPONSE_TOO_LARGE，不返回空列表冒充成功。SDK 输出 Schema 保留 integer、非负约束及上下限，使用 Schemars 原生 transform 去掉默认 TS 验证器不识别的 uint32 / uint64 格式注解，不削弱数据校验。

## 验证

复验命令见 [开发文档](development.md#wb-204-七项只读工具验证)。Core 契约测试覆盖桌面结果一致、分页、计数、旧版本、修订冲突、跨资产引用、中文 / emoji / 组合字符 / ZWJ / NUL、边界和公开字段；官方 TS 客户端通过真实 stdio 与 Windows 管道读取合成资料，并覆盖超大请求 / 结果和原有握手、关闭、断连及自然进程回收。测试全部使用隔离资料库，不接触真实媒体、设置或凭据。
