# 素材库检索与桌面命令契约

WB-202 新增的 Core 本机 IPC、状态与启停命令单独维护于 [本机 IPC 契约](mcp-local-ipc.md)。它复用本页分类 / 标签读取服务，不把桌面 DTO 直接作为未来 MCP 的公开资料接口。

对应 [WB-111、WB-112、WB-114、WB-115、WB-116、WB-117、WB-118](workbench-knowledge-base-tasks.md)，实现于 `library/search.rs`、`library/views.rs`、`library_api.rs`、`library_commands.rs`、`src/lib/library-api.ts` 及 `library_transfer.rs` / `library/backup.rs` / `library/content_export.rs`。本页说明本地桌面 API；WB-113～WB-118 已接入知识库列表、分类标签、资产详情、文件播放、正文编辑、笔记、基础工作台和导出备份恢复，MCP 读取及受控新建已由 WB-204～WB-310 接入，专项边界见[受控写入](mcp-write-tools.md)。

## 主动收藏与任务关联

`library_assets.archived_at` 为 NULL 表示已收录，非 NULL 表示未收录或已移出。新下载成功仍自动登记资产、文件与 `asset_task_links`，新资产默认未收录；同平台、同作品 ID 的不同清晰度复用原资产，新增文件版本不改变其收藏状态。升级不回写既有资产的 archived_at。手动导入本地视频主动收录，已登记本地文件再次导入也会收录。

桌面新增 `get_task_collection_states({taskIds}) -> [{task_id,asset_id,collected}]`，一次 SQL 批量解析任务，未登记任务省略。`set_task_collected({taskId,collected})` 返回相同状态对象，经已有任务关联更新原资产；重复设置幂等。每批最多 10,000 个非空任务 ID。`archive_library_asset` 保持原契约，重复同状态不改写时间。移出不删除媒体、笔记或文案，清理下载历史不删除资产资料。schema v11、逻辑包 v9 不变，历史高级资料和恢复能力保留。

## 1. 检索

列表结果新增 category_ids、tag_ids 和 has_transcript 摘要，搜索命中的 document_id / note_id 可直接打开相应文案版本或笔记。前端通过 `libraryApi.search(request)` 调用 `search_library_assets`。外层 Tauri 参数为 `{request}`；request 内字段保持 Rust DTO 的 snake_case，缺省值如下：

```json
{
  "query": "",
  "category_ids": [],
  "tag_ids": [],
  "platform": null,
  "archived": false,
  "uncategorized": false,
  "unorganized": false,
  "file_state": "all",
  "cursor": null,
  "limit": 20
}
```

- 中文单字、短词和普通子串均可匹配；`%`、`_`、引号及反斜杠按普通字符处理，不作为通配或 SQL。去除查询首尾空白，查询原始长度上限 512 个 Unicode 字符；英文采用 SQLite 原生 ASCII 大小写转换，不承诺跨语言分词或语义搜索。
- 搜索标题、作者、全部保留版本的识别原文 / 整理稿，以及笔记正文。每资产返回一个优先命中，顺序为标题、作者、整理稿、识别原文、笔记；同一文案类型优先较新的匹配版本，不把历史命中伪装成当前稿。
- 多个一级分类是 OR，仅匹配直接关联，不包含历史后代；多个标签是 AND。不同维度之间是 AND，重复标签和重叠分类不使资产重复。`uncategorized=true` 不能同时指定分类。
- `platform` 支持 `douyin`、`tiktok`、`youtube`、`telegram`、`local`；`null` 不限平台，`local` 匹配无平台身份的资产。`archived=false` 只返回在库资产，`true` 只返回已移出的资产。
- `unorganized=true` 仅返回尚无整理稿版本且没有任何笔记的资产；识别原文不改变此状态，与分类及标签等其他筛选按 AND 组合。
- `file_state` 为 `all`、`available`、`missing`、`changed`。至少一个文件存在即 available；所有文件不可用或没有文件即 missing；至少一个现存文件与登记指纹不符即 changed。available 与 changed 可重叠，结果附实际文件计数。
- 文件状态查询时检查本机文件系统，筛选在分页前执行；不新增文件监视器，也不承诺自动感知应用外的所有文件变化。
- `limit` 范围 1～100。按 `created_at DESC, id DESC` 使用键集分页，响应包含 `next_cursor={created_at,id}` 或 `null`。后续请求原样使用游标；修改筛选时清空游标。分页固定顺序，不冻结活跃资料库和文件系统的快照。

响应 `items` 包含资产元数据、文件计数和至多 120 个 Unicode 字符的命中摘要。文案命中带 `document_id` / `document_version`，笔记命中带 `note_id`；全文由独立读取命令获取。不存在的分类 / 标签返回对应错误，不悄悄忽略筛选。

## 2. 命令与前端映射

除嵌套 DTO 外，Tauri 参数使用 camelCase，例如 Rust 的 `asset_id` 对应 `{assetId}`。完整类型定义以 `src/lib/library-api.ts` 为准，公共对象由 `src/api.ts` 导出。

| 原生命令 | `libraryApi` 方法 | 结果 / 行为 |
| --- | --- | --- |
| `search_library_assets` | `search` | 分页资产与命中摘要；使用 Tauri 异步调度，避免在同步原生处理路径执行全文扫描 |
| `get_workbench` | `workbench` | 最近访问、待分类、待整理摘要和真实操作；异步调度 |
| `record_library_visit` | `visit` | 成功打开资产后记录一次访问，后台读取不计入 |
| `get_library_asset` | `asset` | 资产、文件状态、分类 / 标签 ID、文案与笔记摘要 |
| `archive_library_asset` | `archive` | 移出知识库或恢复，不删除文件 |
| `list_library_categories` | `categories` | 分类列表，包含父 ID 和排序值 |
| `create_library_category` | `createCategory` | 新分类 ID |
| `update_library_category` | `updateCategory` | 改名、移动和排序，拒绝循环与同级重名 |
| `delete_library_category` | `deleteCategory` | 删除分类关系；存在子分类时拒绝，不删除资产 |
| `set_library_categories` | `setCategories` | 按资产 ID 批量添加 / 移除一个分类关系，批次原子提交 |
| `list_library_tags` | `tags` | 标签列表 |
| `create_library_tag` | `createTag` | 新建或返回同名标签 ID |
| `rename_library_tag` | `renameTag` | 标签改名，拒绝重名 |
| `delete_library_tag` | `deleteTag` | 删除标签及关系，不删除资产 |
| `set_library_tags` | `setTags` | 原子批量添加 / 移除一个标签关系 |
| `list_library_documents` | `documents` | 文案版本摘要，不含正文 |
| `get_library_document` | `document` | 指定资产所属版本的正文、原始来源和模型信息 |
| `save_library_document` | `saveDocument` | 使用 `expectedVersion` 新增整理稿版本，冲突不覆盖；可指定同资产的 `sourceDocumentId` |
| `list_library_notes` | `notes` | 笔记摘要及修订号，不含正文 |
| `get_library_note` | `note` | 指定资产所属的笔记与引用 |
| `create_library_note` | `createNote` | 人工笔记，校验引用，返回 ID；输入只接受 kind / body / citations |
| `edit_library_note` | `editNote` | 以 `expectedRevision` 修改正文，错误归属或过期修订不写入 |
| `get_library_cover` | `cover` | 当前资产持有的封面字节 |
| `prepare_library_playback` | `playbackUrl` | 检查所属文件与可用性后开放单文件资源范围，返回可用于播放器的 URL |
| `reveal_library_file` | `reveal` | 检查所属文件后调用系统文件定位，不接受任意路径 |
| `open_library_file` | `open` | WB-114 新增；重新检查所属普通文件后调用系统默认播放器，失败为 `PLAYBACK_OPEN_FAILED` |
| `export_library_document` | `exportDocument` | WB-115 新增；按资产 / 文案 ID 导出已保存版本为 TXT，原生选择器取消返回 `null`，成功返回路径 |

此前 `api.importLibraryVideo`、`relinkLibraryFile`、`cancelLibraryImport` 和资产转录方法继续有效。单篇 TXT 已可用；WB-117 的 Markdown / JSON、逻辑备份和恢复提供 Rust Core 接口，WB-118 已接入格式 / 目录选择器、后台进度和设置界面；不新增永久删除资产或识别原文的接口。

## 3. 摘要、全文和错误

资产详情只包含文案、笔记摘要；全文仅通过指定资产与内容 ID 读取，不批量传输所有版本正文。文案来源变化对照当前关联文件与原始来源指纹；当前文件变化或缺失不会删除已有正文。无法确定来源关系时 `source_changed=null`，不虚构未变化。

桌面编辑器按需读取来源版本链以核对整理稿原文，链中版本不可变。`saveDocument` 的 `expectedVersion` 对照资产全部整理稿的最新版本，成功新增一版；`editNote` 的 `expectedRevision` 对照指定笔记修订号，冲突返回明确错误。保存后读取失败不撤销已经成功的写入，UI 提供独立重读。新笔记由 Core 固定为人工来源，引用由 Core 校验资产 / 文档 ID 和原句，不允许输入伪造 AI 来源。

`exportDocument(assetId,documentId)` 只导出该资产所选的已保存文案，不接收前端正文或媒体路径。Rust 复用旧转录 TXT 导出的 UTF-8 BOM、平台换行及临时文件提交；保护资产全部现关联文件和历史原文来源路径，拒绝同路径及既存硬链接别名，返回 `TRANSCRIPTION_EXPORT_SOURCE`。导出不写文案版本，也不发送成功的资料变更事件。

资产的 `created_at` / `updated_at` / `archived_at` 为 UTC 秒时间戳，展示时转换为毫秒；文件 `duration_ms` 为毫秒，`mtime_ns` 为字符串纳秒指纹。播放、定位和系统打开均传入 `{assetId,fileId}`，归属不符或文件缺失由 Rust 明确拒绝。重新关联沿用 `api.relinkLibraryFile`，成功保持文件 ID、更新路径和探测规格；选择器取消返回 `null`，已开始操作的取消返回 `CANCELED`。

写入复用 Rust 领域服务和现有事务。SQL / 存储 JSON 错误对桌面端规范为 `DATABASE_ERROR`，保留内部错误链；不存在、重名、循环、版本冲突和引用错误保持各自错误码。`src/api.ts` 提供中文反馈。DTO 反序列化错误由 Tauri 拒绝，检索 DTO 拒绝未知字段和非法枚举。

不得以这些命令读取配置或凭据，也不返回平台签名媒体地址。桌面文件状态及来源详情可以包含已登记本地路径；MCP 的路径隐藏及范围限制由其专门契约另行实施，不将桌面接口直接视为 MCP 输出。

## 4. 变更事件

`libraryApi.subscribe` 监听 `library_changed`，只包含：

```json
{"scope":"documents","asset_ids":["稳定资产ID"]}
```

scope 为 `assets`、`categories`、`tags`、`documents`、`notes`、`visits` 或 `operations`。空 ID 列表表示该范围需要整体刷新，例如分类改名 / 删除会影响组织树与资产分类展示；非空列表可以按资产刷新相关详情及查询。

资料写入和访问保存提交成功后才通知；`operations` 则在真实操作状态变化时通知。新下载入库、导入、重新关联、转录成功和旧下载入口的文件删除 / 历史清理同样接通此事件；失败不伪装成功。事件不包含全文、凭据或任意文件内容，转录进度仍通过已有的状态摘要事件发送。新页面负责订阅、退订、处理失效查询与展示持续错误。

## 5. 基础工作台

`get_workbench` 返回 `recent`、`uncategorized`、`unorganized`，每项为 `{items: LibraryAsset[], total}`。每区最多六个资产，待办按创建时间 / ID 倒序，最近访问按持久化访问序号倒序；总数涵盖该区全部在库资产。待分类和待整理的“查看全部”使用同一检索条件，不另建状态标志。归档资产不出现在这三项，恢复后重新参与计算。

schema v3 新增 `asset_visits`，按资产保存 UTC 秒级访问时间及单调序号，确保同秒连续访问也保留顺序；重复访问只更新一条记录。v1 / v2 升级前使用包含 WAL 的一致性备份，DDL 与版本推进在同一事务完成。`get_library_asset`、分类批量读取、后台事件刷新不写访问；详情首次成功读取后独立调用 `record_library_visit`，离开后再进入会更新。访问保存失败不会阻止阅读，提供单独重试。

`operations` 每项为 `{id, kind, title, state, phase, error, asset_id, file_id, task_id}`。kind 为 `download` / `transcription` / `import`；不含正文、媒体 URL 或本地路径。下载来源是既有 Core 任务，包含解析、排队、下载及失败 / 入库警告；转录包含运行、失败及中断摘要，按同一文件用内存未落盘状态和活动操作覆盖旧数据库摘要。导入只有实际受理探测后才出现，选择对话框等待不计为运行；本次进程保留最近一次失败，下一次开始时清除，取消不计为失败。

界面将活动操作与“需要处理”分开，失败原因持续展示。下载入口重读真实任务后按 ID 显示该项，绕过之前的平台、状态、搜索和列表分页；已移除任务留在工作台并反馈。资产转录 / 重新关联按资产与文件 ID 打开详情；导入进入持续挂载的知识库导入入口。`operations` 事件表示操作状态变化（可包含结束或失败），不表示资料提交成功；`visits` 仅在访问写入成功后发送。前端订阅资料变更与转录阶段 / 终态，下载状态变化或窗口聚焦也重新读取；刷新失败保留上次结果并显示重试，不用演示数据兜底。

## 6. 单篇导出与逻辑包核心（WB-117）

本节是 Rust Core 契约。既有 `export_library_document` 命令仍只导出 TXT；WB-118 的新格式、备份恢复选择器及页面由下节所列命令接入。

| Core 方法 | 输入 | 结果 |
| --- | --- | --- |
| `export_library_content` | 资产 ID、`ContentSelection::Document(id)` / `Note(id)`、`ContentFormat::Txt` / `Markdown` / `Json`、输出文件路径 | 导出已保存内容，不新增版本或事件 |
| `backup_library` | 新目录路径、`include_media: bool`、进度回调 | `BackupReport {assets, files, missing_media, bytes}` |
| `restore_library` | 逻辑包目录、新恢复目录、进度回调 | `RestoreReport {assets, documents, notes, missing_media}`，成功发送一次 `library_changed`，scope=`assets`、asset_ids=[]，表示重新读取整个资料视图 |

### 单篇格式

TXT 保持 UTF-8 BOM 与平台换行；旧入口与新入口共用已有的临时文件提交及源文件保护。Markdown 使用正文和来源信息区，包含资产、版本 / 修订、模型、源文档 ID 等 JSON 元数据；所选整理稿及笔记引用会追溯不可变来源链，附逐版来源 / 模型摘要而不重复全文。JSON 为无 BOM 的 UTF-8，外层 `{format:"yingqu-content", version:1, asset, selection, content, references}`；笔记引用附被引用资产、文案版本摘要和原句。单篇导出可以包含原始文案来源路径；它不是脱敏后的 MCP 响应，也不是整库恢复格式。

内容来自所属资产的已保存文档 / 笔记，不接受调用方提供的正文。新入口保护库内所有当前媒体与历史文案来源的同路径和硬链接别名，防止输出覆盖引用来源；失败保留原目标，且不发送资料变更事件。导出目标可按用户选择覆盖原有普通导出文件，保持原生保存流程语义。

### 目录包格式 v2（读取兼容 v1）

使用目录包，文件结构如下；整个目录需要一起复制：

```text
backup/
  manifest.json
  manifest.sha256
  files/
    00000000.bin
    00000001.bin
```

`manifest.json` 的 `format` 为 `yingqu-library`，`version` 为 9（reader 同时接受旧 v1 / v2 / v3 / v4 / v5 / v6 / v7 / v8），另含 `created_at`、`include_media` 以及下列白名单集合：

- `assets`：在库及归档资产、平台身份、规范来源链接、创建 / 更新时间。
- `files`：稳定文件与资产 ID、原始文件名、登记规格 / 指纹，以及可选媒体 payload；不携带原始绝对路径。
- `categories`、`tags`、`category_links`、`tag_links`：完整组织结构、排序和关系。
- `documents`：全部不可变版本、revision、来源文档 ID、模型 / 流水线信息、正文及可迁移来源指纹。
- `notes`：笔记原有 ID、revision、origin / model、正文和精确引用；v5 可选 analysis，包含结构化字段、资源引用快照和 human_revised；v6 可选 comparison，保存逐结论来源快照。
- `clips`：原视频片段引用、起止、标题 / 理由、不可变文案版本 / 原句与可移植 Source，v3 新增。
- `keyframes`：PNG payload、采样元数据、片段绑定及可移植 Source，v4 新增；无媒体包也包含图片。
- `research_topics`、`topic_members`：专题正文、修订、三类成员引用和名称快照、完整成员顺序，v7 新增；v9 专题增加可选 ai_source，保留声明的模型名称与精确文案引用。
- `ideas`、`idea_references`：自己的标题 / 观点 / 草稿、修订、三类参考和名称快照 / 顺序，v8 新增；v9 草稿增加可选 ai_source，人工编辑保留 AI 创建来源。
- `resources`：封面 payload；`visits`：持久化访问顺序与时间。

旧包缺 ai_source 按人工来源读取；v1～v8 包夹带实际 AI 元数据拒绝。v9 恢复时校验模型名称、引用的资产与文案归属及精确原句。

payload 为 `{path, size, sha256}`。路径固定为 `files/` 下八位十进制编号加 `.bin`，独立于标题、用户文件名及 ID。`manifest.sha256` 是 manifest 原始字节的 64 位小写 SHA-256 十六进制文本；每个 payload 另有大小与 SHA-256。校验用于发现损坏，不是包的来源认证。manifest 上限 256 MiB，单封面上限 16 MiB，单关键帧 PNG 上限 8 MiB、边长最多 1280；大视频按 64 KiB 流式复制、校验，不整体载入内存。

包不读取或包含应用设置、Cookie、代理凭据、下载任务 / 历史关联、运行中操作或 ASR 暂态摘要。用户文案和笔记按原文保留。视频可选择不打包，`media=null` 表示未随包提供；完整模式遇到已缺失的视频也保留文件引用，并计入 `missing_media`。报告中该计数表示恢复时没有随包媒体的文件数，包含主动排除的媒体；`files` 是实际随包视频数，`bytes` 是封面、关键帧及视频 payload 字节总数。

包内原文来源使用文件名、大小、mtime 和是否对应当前关联文件表示。恢复后的实际文件路径一律位于新恢复目录；原始绝对路径不会被拿来打开或复制本机文件。随包且指纹吻合的视频会更新保存位置和新位置的 mtime，保持识别原文与该文件的关联；之前已变化 / 已重新关联的来源继续显示变化，不将旧文案伪装成新视频的内容。缺媒体的引用恢复为目录内占位路径，仍可读文案、编辑与手工重新关联。历史来源使用目录内不可用引用，不借助外部机器路径恢复。

### 恢复、失败与取消

恢复要求知识库完全为空，连已有的分类 / 标签 / 空专题 / 独立选题也拒绝覆盖；不要求应用设置或非视频下载记录为空。Core 同时拒绝解析、排队、下载、导入或转录活动中的恢复。准备前和提交前均检查，准备期间其他操作新增的资料不会被覆盖。

先校验版本、清单摘要、文件路径 / 大小 / 摘要及关系。在隔离内存库用当前 schema 验证外键、七类实体稳定 ID 非空、同资产归属、分类与文档来源循环、笔记引用原句及片段的版本 / 范围 / 来源链指纹等，再准备文件并事务插入。目标目录必须不存在，拒绝覆盖现有文件、目录或链接；包内路径不能向上跳转、使用绝对路径、反斜杠或 NTFS ADS，目录联接 / 符号链接也不能将 payload 读取转向包外。实际恢复名由内部编号和视频扩展名白名单生成，不恢复可执行扩展名。

包与恢复目录在同级临时目录中准备，失败 / 取消回收本次暂存；所有关系在同一 SQLite 事务提交。发布目录后若数据库提交失败，回滚数据库并移除刚创建的目录；清理本身失败会显式报告 `LIBRARY_RESTORE_CLEANUP_FAILED`，不宣称成功。这不是跨文件系统与数据库的通用崩溃恢复协议；意外断电或强制杀进程的暂存清理需在原生阶段另验，不声称两种存储拥有共同事务。

`TransferProgress {phase, completed_files, total_files, bytes}` 的 phase 为 `copying` / `validating` / `committing`，完成以返回报告为准。长复制不持有 Core 状态锁，每 64 KiB 调用回调；调用方可返回 `CANCELED` 取消，Core 同时检查关闭状态。这些是同步核心回调；WB-118 的操作包装在既有 Core worker 生命周期中异步执行，接入原生进度与取消，不新建第二套持久化任务队列。

稳定错误包括 `LIBRARY_NOT_EMPTY`、`LIBRARY_BUSY`、`LIBRARY_TARGET_EXISTS`、`LIBRARY_PACKAGE_VERSION`、`LIBRARY_PACKAGE_INVALID`、`LIBRARY_PACKAGE_PATH`、`LIBRARY_PACKAGE_CHECKSUM`、`LIBRARY_PACKAGE_TOO_LARGE`、`LIBRARY_BACKUP_SOURCE_CHANGED`、`LIBRARY_RESTORE_CLEANUP_FAILED`，复用 `CANCELED`、`IO_ERROR`、`DATABASE_ERROR` 和单篇 ID / 源文件保护错误。


## 7. 原生导出、备份恢复界面（WB-118）

| 命令 / `libraryApi` | 输入及结果 |
| --- | --- |
| `export_library_content` / `exportContent` | `{assetId, selection:{kind:"document"或"note",id}, format:"txt"或"markdown"或"json"}`；按所属 ID 选择已保存版本，再用对应扩展名与过滤器另存为；取消返回 null，成功返回实际路径 |
| `get_library_backup_info` / `backupInfo` | `{assets,documents,notes,files,available_files,media_bytes,empty,busy}`；统计包含在库 / 归档资料，媒体体量来自当前本机文件，busy 指阻止恢复的下载 / 导入 / 转录活动 |
| `get_library_transfer` / `transfer` | 当前 / 最近一次 `TransferSnapshot`，本次进程尚无操作时 null |
| `start_library_backup` / `startBackup` | `{includeMedia}`，界面默认 false；选择父目录后由 Rust 新建带时间与 UUID 后缀的子目录，取消返回 null，否则返回初始操作快照 |
| `start_library_restore` / `startRestore` | 无前端路径参数；先查空库 / 活动状态，再依次选择逻辑包目录与恢复父目录，任一步取消返回 null，否则新建恢复子目录并返回快照 |
| `cancel_library_transfer` / `cancelTransfer` | `{operationId}`，只取消匹配的当前操作；提交阶段拒绝取消，终态调用不改写成功结果 |

`TransferSnapshot` 为 `{id,order,sequence,kind,include_media,state,phase,completed_files,total_files,bytes,canceling,path,error,result}`。kind 为 backup / restore，state 为 running / completed / failed / canceled；phase 追加 preparing 后沿用核心 copying / validating / committing。result 为相应核心报告；path 由原生选择器与 Rust 生成，不信任前端任意输出路径。

每个新操作递增 order，每次更新递增 sequence；`library_transfer_updated` 发送同一 DTO。前端按 order / sequence 合并事件、初始读取和启动返回值，旧操作的高 sequence 也不能覆盖新操作。进度只在阶段 / 文件数变化或间隔达到 100ms 时发送，底层仍每 64 KiB 检查取消；终态始终发送，完成以真实结果为准。状态保留在 Core 本次进程内，切页后可读取，重启不伪造续传。

Core 同时只允许一个备份 / 恢复 worker。实际复制在 spawn_blocking 执行，外层 worker 登记到既有 workers；关闭应用发出取消并等待实际工作结束。取消检查与进入 committing 使用同一现有 Core 临界区，已接受的取消不会被随后的提交吞掉。备份可读取活动资料快照；恢复仍执行准备前、提交前的空库与活动状态检查。目录选择等待不计入运行操作，不启动进程或 Core 复制；错误与取消不发成功的资料变更事件。

设置页提供仅资料 / 包含视频范围、真实数量与可用视频体量；非空库或活动任务阻止恢复，失败原因持续可见。成功显示保存位置、数量和未随包媒体数；恢复后资料视图响应已有 library_changed 事件。工作台将运行 / 失败的 backup / restore 路由至设置的知识库分组。订阅失败与读取失败分别保留并可重新连接，不用成功的状态读取掩盖订阅断开。

旧 TXT 命令继续兼容；新文稿 / 笔记 UI 统一调用 `exportContent`。保存选择器期间禁用编辑 / 版本切换，未保存内容不能导出；保存 / 另存为的系统交互仍需在真实目标平台验收。应用开发验证与真实原生对话框结果分别记录，不以 IPC mock 或桌面特性编译代替 GUI 通过。


### WB-302 语音段时间

`get_library_document` 的内容响应新增可选 `timing`，旧文案 / 整理稿为 null。它包含 artifact schema 1、媒体 PTS 零点、VAD 来源 / 修订、语音段精度、采样率 / 帧数、操作 ID 和有序段落；结合既有 summary 的文档 / 版本 / 文件 ID、Source 及 source_changed 判断绑定。列表摘要和进度仍不返回全文段落，不扩大 MCP 范围。

内部 `transcribed` 增加固定 JSON 文件描述符，engine 0.16.0 / JSONL v3，Rust 检查后与文档 / 操作同事务提交。schema v4 升级前保留一致性备份，旧文案不补造时间。备份 v2 的文档可携带 timing，恢复仍需验证整体关系；旧 v1 允许没有 timing，v1 混入实际时间数据拒绝。详细字段、版本 / 文件绑定、媒体范围及平台边界见 [段落契约](transcript-segments.md)。


## 9. 片段收藏与范围播放（WB-303）

`list_library_clips`、`create_library_clip`、`delete_library_clip`、`prepare_library_clip` 已注册，通过同一 Core 校验资产 / 文件及 Source。`LibraryChange.scope` 增加 `clips`；创建、删除成功后通知，失败不通知。片段保存引用，不裁剪媒体。详细 DTO、标题 / 理由 / 范围边界、不可变文案引用、播放器实时能力和 schema v5 / 备份 v3 兼容见[片段契约](asset-clips.md)。MCP 范围保持现有只读工具及受限 Resources。

## WB-304 关键帧命令与事件

| 命令 | 参数与结果 |
| --- | --- |
| `list_library_keyframes` | `assetId` → 元数据数组，不包含 PNG |
| `get_library_keyframe_image` | `assetId,keyframeId` → 二进制 `tauri::ipc::Response`，前端得到 ArrayBuffer |
| `delete_library_keyframe` | `assetId,keyframeId` → 删除资源，不删除片段或源文件 |
| `get_library_keyframe_operation` | 无参数 → 当前 / 最近一次快照或 null |
| `start_library_keyframes` | `assetId,request` → 真实操作快照；异步 Tauri 命令，在运行时注册 Core worker |
| `cancel_library_keyframes` | `operationId` → 等待进程回收和本批清理完成 |

request 为 `{kind:"position",file_id,position_ms}` 或 `{kind:"clip",clip_id,count}`，禁止任意路径。快照含 `id,sequence,asset_id,file_id,clip_id,state,completed,total,canceling,error,frame_ids`。state 为 running / completed / failed / canceled；`library_keyframes_updated` 推送同一快照。跨操作迟到事件不能覆盖当前操作；进度不返回图片。`LibraryChange.scope` 新增 keyframes，完成 / 删除后刷新列表；操作状态沿现有 operations 事件接入工作台。

摘要含批次、请求位置、原始 PTS / timebase、actual_ms、采样区间 / 序号 / 总数、尺寸、黑帧、bytes、missing / source_changed。错误包括 KEYFRAME_BUSY、KEYFRAME_RANGE_INVALID、KEYFRAME_RANGE_UNAVAILABLE、KEYFRAME_SOURCE_CHANGED、KEYFRAME_INVALID_RESULT，沿现有错误传播。图片读取与删除均重新校验资产归属。

WB-304 交付时 schema v6，engine 0.17.0 / JSONL v3；逻辑备份 writer v4、reader v1～v4，`keyframes` 保存元数据、可移植 Source 与 PNG payload，即使排除视频仍保留图片；bytes 包含图片字节。详细采样、PNG 限制和恢复校验见[关键帧契约](asset-keyframes.md)。本批不扩展 MCP 工具 / Resources。

## WB-305 播放器与同步阅读

不新增 Tauri / MCP 命令或持久化字段。`LibraryVideoHandle` 在 clock / toggle 基础上新增 `seek(positionMs): Promise<void>` 与 `subscribe(callback): unsubscribe`；订阅返回当前文件的 current_ms、真实 ranges 和 paused，媒体释放时发布 null 并清除监听。播放器初始 `positionMs` 仅用于当前来源绑定会话，不接受外部路径。

阅读器使用既有 `get_library_document.timing`，核对 summary.file_id / Source / source_changed 与当前文件后才启用同步。跳转沿用媒体下界探测和 seek 完成 / 超时检查，连续请求取消旧定位。段落片段选择传递 `{citation,range}` 到现有 ClipDraft，最终仍调用 `create_library_clip` 校验并保存；没有第二套片段服务。WB-305 交付时 schema v6 / backup v4 / engine 0.17.0 不变，完整交互和键盘契约见[同步阅读](transcript-sync-reading.md)。

## WB-306 结构化笔记

WB-306 交付时 schema v7、逻辑包 writer v5 / reader v1～v5；WB-307 交付时为 schema v8 / writer v6 / reader v1～v6，WB-308 交付时为 schema v9 / writer v7 / reader v1～v7，WB-309 交付时为 schema v10 / writer v8 / reader v1～v8，WB-310 当前为 schema v11 / writer v9 / reader v1～v9；engine 0.17.0 / JSONL v3 不变。旧批次所述 v6 / v4 是当批契约。

| 命令 | 参数 / 结果 |
| --- | --- |
| `create_library_analysis` | `assetId,input:{content,origin,model_name,citations}` → note ID；AI 手动记录要求模型名 |
| `edit_library_analysis` | `assetId,noteId,content,expectedRevision` → 同一事务更新正文 / 结构和修订号；不接收来源修改 |

content 为 `{kind:"copywriting",opening,viewpoint,evidence,turn,ending,takeaways}` 或 `{kind:"technique",observations:[{dimension,evidence,text,clip_id,keyframe_id}]}`。dimension 为 visual / captions / transition / sound，观察 evidence 为 video / audio / keyframe / unverified，keyframe_id 可为 null。创建时 Core 从既有资源派生 file_id、片段标题 / 范围和 frame_actual_ms，不信任客户端提供的快照；使用精确 PTS 验证范围，关联图与原 Source 一致。

Note 增加可选 `analysis:{content,human_revised}`，技术观察读取时带引用快照。新 kind 为 copywriting_analysis / technique_analysis，摘要沿既有 NoteSummary，正文由结构投影产生。旧笔记不变，新结构笔记不能通过旧纯文本编辑命令改散；返回 ANALYSIS_REQUIRED。其他错误包括 ANALYSIS_INVALID、ANALYSIS_MODEL_REQUIRED、ANALYSIS_REFERENCE_INVALID、ANALYSIS_EVIDENCE_INVALID，复用 NOTE_CONFLICT 和 notes 变更事件。保存成功后回读失败不重建记录，冲突副本包含结构 / 来源 / 模型 / 引用。

备份 v5 对结构、正文、来源和资源关系统一验证；删除的引用以不可用快照保留，不自动关联新资源。创建来源不可变，人工修改 AI 笔记的 human_revised 由 Core 设置。P1B 工具 / Resources 及内部 IPC 方法白名单保持原范围，只读正文可直接包含拆解投影，不开放本批 Tauri 写命令。完整行为及限制见[拆解契约](structured-analysis.md)。

## 跨视频比较（WB-307）

新增 `get_library_comparison_source(pin)`、`create_library_comparison(assetId,input)` 和 `edit_library_comparison(assetId,noteId,input,expectedRevision)`。完整 Note 增加可空 comparison，普通笔记不受影响。逐结论保存独立来源快照，Core 校验资产 / 文案版本 / 拆解修订 / 片段范围并生成正文与 citations；通用正文编辑拒绝比较记录。schema v8 / writer v6 / reader v1～v6，MCP 白名单保持只读。入参、缺失语义和恢复校验见[比较契约](video-comparison.md)。

## 研究专题（WB-308）

新增 `list_library_topics`、`get_library_topic`、`create_library_topic`、`edit_library_topic`、`add_library_topic_member`、`remove_library_topic_member` 和 `reorder_library_topic_members` 七项桌面命令。成员组合 video / clip / note，写入使用专题 revision CAS，排序接收完整 member ID 全排列。来源缺失 / 归档保留引用，移除仅解除成员关系；备份新增 research_topics/topic_members，schema9 / writer7 / reader1～7，空专题库也拒绝恢复覆盖。BackupInfo / BackupReport / RestoreReport 增加 topics 数量；library_changed 增加 topics scope。准确命令名、结构、错误、读取 / 恢复语义见[专题契约](research-topics.md)。MCP 白名单不增加。

## 自己的选题与草稿（WB-309）

新增 `list_library_ideas`、`get_library_idea`、`prepare_library_idea_reference`、`create_library_idea`、`edit_library_idea` 桌面命令。自己的 title / viewpoint / body 与完整 references 一起通过 revision CAS 保存；新来源校验存在及归属，已有缺失引用可保持。工作台摘要增加 ideas 列表 / 总数，不含观点或正文；library_changed 增加 ideas scope，BackupInfo / BackupReport / RestoreReport 增加 ideas 数量。WB-309 交付时 schema10 / writer8 / reader1～8，当前 WB-310 schema11 / writer9 / reader1～9，具体 DTO、来源往返、冲突副本、迁移和恢复见[选题契约](ideas-and-drafts.md)。没有增加 MCP 白名单。

## WB-310 受控写入契约

当前 schema11 / writer9 / reader1～9，为专题和草稿新增只读 ai_source。桌面 get_mcp_connection 返回 write_enabled，set_mcp_write_enabled 管理本次服务会话授权；IPC 增加六项固定写入映射，不能更改授权、已有正文、删除或设置。输入、DTO、事务、错误、AI 来源和平台边界以[受控写入契约](mcp-write-tools.md)为准。

## WB-311 MCP 转录长操作

新增三项固定 IPC映射，start / cancel沿用独立写授权，get按真实 operation ID只读查询；复用原转录worker、持久摘要和进程回收。立即返回的 running + cancel_requested 不等于实际取消完成；工具目录16项，schema11 / writer9 / engine0.17.0 不变。字段、缓存 / 操作保留范围、超时和断连规则见[转录长操作](mcp-transcription-operations.md)。

## 一级分类接口

桌面分类读取的 `parent_id` 固定为 null；新建、更新接口保留旧参数形状，但非空 parent 返回 `INVALID_INPUT`。MCP `create_category` 仅接受省略或 null 的 `parent_id`，分类读取同样平铺。底层旧层级、稳定 ID、关联及备份可恢复信息继续保留，不执行自动迁移。分类筛选仅查询直接关联。列表内创建新分类，不再在管理弹窗中新建；已移出和导入入口隐藏，原有资产状态及导入服务保留。
