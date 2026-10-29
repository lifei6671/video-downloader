# 工作台、视频知识库与 MCP 实施任务清单

日期：2026-10-29。

依据：[技术方案](workbench-knowledge-base-plan.md)。本清单用于安排实施和记录进度，不扩大方案范围；已开始准备与首批核心开发，实际验证见 [实施记录](workbench-knowledge-base-validation.md)。未完成的项目保持未勾选。

## 使用方式

- 每项使用稳定编号，后续可直接指定“执行 WB-101”。从依赖已满足的任务开始，交付物和验收条件均满足后将 `- [ ]` 改为 `- [x]`。
- 进行中或阻塞时保持未勾选，在该项下追加“状态：进行中 / 阻塞；原因：……；下一步：……”。不使用勾选表示已经开始。
- 完成时在任务下补充提交或文件位置、实际验证命令与结果，以及必要的验收记录链接。一个任务可以跨多个提交，不要求逐项提交。
- 每项包含自己的相关测试和文档同步；阶段验收负责组合流程、真实客户端和平台验证，不代替任务级检查。
- `NOT_RUN` / `UNAVAILABLE` 如实记录，不能计为通过。阶段必需验收未满足时保留未勾选；可继续不依赖缺失条件的工作，但不得声明该阶段完整交付。
- 新任务开始前核对当前分支、工作区、作用域规则和相关源码，保留已有修改。实施、提交、发布分别遵循当次授权，本清单本身不代表提交或发布授权。
- 产品和数据约定以技术方案为准，视觉 token 与交互细节以 [DESIGN.md](../DESIGN.md) 为准。范围变化先同步对应方案，再调整任务。

## 阶段顺序

| 阶段 | 编号范围 | 完成后可用能力 | 阶段完成项 |
| --- | --- | --- | --- |
| 准备 | WB-001～WB-003 | 明确现有调用路径、页面契约和测试样本 | 三项全部完成 |
| P1A | WB-101～WB-119 | 视频入库、分类、文案笔记、检索、工作台和备份 | WB-119 |
| P1B | WB-201～WB-208 | 本机 AI 读取和总结知识库 | WB-208 |
| P2 | WB-301～WB-313 | 片段、拆解、专题、选题及 AI 写回 | WB-313 |
| P3 | WB-401～WB-406 | 根据使用证据确定下一轮目标 | 各项评估独立完成，不代表功能实现 |

准备项与 WB-101～WB-118 共21项已完成；WB-401使用后评估完成，主清单为 **22 / 49**。WB-119、WB-201～WB-208、WB-301～WB-311 已完成 Windows 实现与对应验收；跨平台总项保持未勾选。WB-312 已实现并完成 Apple Silicon 限定验收，Windows / Intel 尚未验收。

2026-10-29：本机 Apple Silicon 已完成 WB-119、WB-201～WB-208、WB-301～WB-313 共 22 个平台子项。WB-301 已补完整长视频与具名真人朗读的真实 CPU / VAD 验证，交付范围仍为语音段级粗定位，全文准确率不计通过；原 Windows 约 298 秒样本的同输入跨平台比较仍为 NOT_RUN。Intel 无可用机器，继续 UNAVAILABLE；不能用 arm64 代验，总计仍为 21 / 49。证据和限制见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)、[第 31 批记录](workbench-knowledge-base-validation.md#第-31-批macos-原生组合验收与取消回收修复)、[第 32 批记录](workbench-knowledge-base-validation.md#第-32-批macos-结构化拆解原生验收)、[第 33 批记录](workbench-knowledge-base-validation.md#第-33-批macos-跨视频对比原生验收)、[第 34 批记录](workbench-knowledge-base-validation.md#第-34-批macos-研究专题原生验收)、[第 35 批记录](workbench-knowledge-base-validation.md#第-35-批macos-选题与草稿原生验收)、[第 36 批记录](workbench-knowledge-base-validation.md#第-36-批macos-片段收藏与范围播放原生验收)、[第 37 批记录](workbench-knowledge-base-validation.md#第-37-批macos-关键帧原生验收与预览焦点修复)、[第 38 批记录](workbench-knowledge-base-validation.md#第-38-批macos-同步阅读原生验收)及[第 39 批记录](workbench-knowledge-base-validation.md#第-39-批macos-长语音时间来源复验)。WB-312 的新增媒体读取与模板证据见[第 40 批记录](workbench-knowledge-base-validation.md#第-40-批wb-312-片段图片与分析模板)。WB-313 的真实视频、六帧模型消费、AI 写回及双备份恢复见[第 41 批记录](workbench-knowledge-base-validation.md#第-41-批wb-313-真实视频学习到创作组合验收)；人工视听只覆盖两段各 8 秒，全文准确与连续视听质量不计通过。WB-401 的使用后评估已完成，见[评估报告](workbench-real-use-assessment.md)；WB-402～WB-406 尚未启动，评估不代表功能已开发。

## 准备：确认实施入口

- [x] **WB-001：核对资产、任务、转录和删除调用路径。** 依赖：无。
  - 完成证据：调用路径及差异已记录于 [实施记录](workbench-knowledge-base-validation.md)。
  - 交付：在实施记录中列出现有数据库初始化、任务提交、封面清理、转录保存、历史清理、文件删除和 IPC 入口，以及本次需要调整的模块。
  - 验收：用当前源码核对技术方案第 2、6、9 节；记录冲突并同步方案，不直接沿用旧行号或旧验收结论。

- [x] **WB-002：补齐知识库页面与操作契约。** 依赖：WB-001。
  - 完成证据：页面、空态、操作和视口契约已写入 [DESIGN.md 第 12 节](../DESIGN.md#12-工作台与知识库页面契约)，尚未实现新页面。
  - 交付：更新 `DESIGN.md` 中导航、知识库列表、资产详情、分类标签管理、工作台和备份入口的布局、文案、空态及键盘行为。
  - 验收：明确三种移除操作、归档恢复、文件缺失、编辑未保存及保存冲突的反馈；复用现有 token 与基础组件，覆盖三个规定视口。

- [x] **WB-003：准备隔离迁移样本和验收记录。** 依赖：WB-001。
  - 完成证据：隔离样本位于 `src-tauri/src/library/tests.rs`，真实 Core 生命周期断言位于 `desktop_core.rs`；验证矩阵见 [实施记录](workbench-knowledge-base-validation.md)。
  - 交付：可复用的测试数据构造入口，包含重复作品、多文件版本、旧模型文案、缺失视频、图集、失败任务和无封面记录；在 `docs/` 建立实际实施验证记录。
  - 验收：使用合成数据或明确隔离的样本，不含真实凭据；记录自动检查、浏览器、原生、安装包及人工验收的独立状态。

## P1A：独立知识库

技术依据：方案第 3～7、9～11 节。所有新增领域服务位于 Rust 核心，不在前端直接操作数据库。

- [x] **WB-101：建立知识库表、约束和版本迁移框架。** 依赖：WB-001、WB-003。
  - 完成证据：`library/schema.sql`、`library/mod.rs` 已实现表结构、外键、版本、WAL 一致性备份及事务回滚；隔离升级测试通过。
  - 交付：方案第 5.1 节中的资产、文件、任务关系、资源、分类标签、文案和笔记表，必要索引、schema 版本及升级前一致性备份。
  - 验收：空库初始化、旧库升级、重复启动、事务回滚和外键约束测试通过；失败不进入可写的新模式，不修改现有设置及下载快照契约。

- [x] **WB-102：实现资产和文件领域服务。** 依赖：WB-101。
  - 完成证据：`library/assets.rs` 已实现平台作品归并、多文件、分页、归档恢复、文件状态和重新关联；相关隔离测试通过。
  - 交付：稳定资产 ID、平台作品归并、多文件关联、列表与详情读取、移出知识库及恢复、文件可用性检查和重新关联。
  - 验收：多分类不复制文件；标题相同但身份不明的内容不误合并；归档默认不出现在列表，文件缺失仍能读取资产资料。

- [x] **WB-103：实现文案版本和笔记存储服务。** 依赖：WB-102。
  - 完成证据：`library/documents.rs` 已实现不可变原文与整理稿版本、来源校验、修订冲突和笔记；相关隔离测试通过。
  - 交付：识别原文、整理稿、人工 / AI 来源笔记、文档版本、修订号、模型信息、源文件指纹和来源引用。
  - 验收：识别原文与整理稿相互独立；旧版本引用可核对；过期修订保存返回冲突，模型未知时不伪造来源。

- [x] **WB-104：实现分类、标签及批量关联服务。** 依赖：WB-102。
  - 完成证据：`library/organization.rs` 已实现分类、标签、排序、循环校验和批量关系；根分类唯一及批量回滚测试通过。
  - 交付：分类新增、重命名、排序、移动、删除，标签管理，以及资产的批量添加和移除关系。
  - 验收：同级分类唯一、根分类同样受约束、禁止循环、父分类包含后代；删除分类或标签不删视频，重复关联无副作用。

- [x] **WB-105：独立管理知识库封面。** 依赖：WB-102。
  - 完成证据：`asset_resources` 持有封面字节；历史清理后封面保留测试通过。
  - 交付：资产拥有的封面或可靠引用，以及读取、持久化和明确的资源清理规则。
  - 验收：删除最后一条相关下载记录后资产封面仍可展示；封面缺失不使视频入库失败，不修改视频原文件。

- [x] **WB-106：迁移历史视频、文案和封面。** 依赖：WB-003、WB-103、WB-105。
  - 完成证据：`library/migration.rs` 与 `Core::open` 已接通历史回填；混合记录、重复执行和失败重试测试通过。
  - 交付：旧任务到资产的稳定映射、历史文件关联、文案及封面迁移，以及升级失败和降级恢复说明。
  - 验收：混合样本迁移正确且可重复检查；已有文案但文件缺失仍可入库；不把图集或失败任务伪装成视频，不重新下载或转录。

- [x] **WB-107：在下载成功提交后自动入库。** 依赖：WB-106。
  - 完成证据：`publish` 已接通成功入库；入库及警告同时写入失败仍保持下载完成，重启补齐的 Core 测试通过。
  - 交付：下载完成与资产建立 / 关联的集成，重复作品关联其资产并保留真实文件版本。
  - 验收：只有校验提交成功的视频入库；仅解析、取消和失败不生成资产；入库失败可见且不会把已提交视频误报为未下载或触发重新下载，重启可根据提交记录补齐关联。

- [x] **WB-108：解除历史清理和文件删除对知识资料的连带删除。** 依赖：WB-103、WB-105、WB-106、WB-107。
  - 完成证据：`remove_history_records` 已接通知识资料保留；历史清理、删除文件、重启和转录文案保留测试通过。
  - 交付：调整单条移除、批量清理和明确删除文件的调用链，解除任务关系但保留知识资料，删除文件后更新文件状态。
  - 验收：清空历史后文案、笔记、分类和封面仍在；文件删除与外部文件缺失均可准确展示；下载中心其余既有行为不变。

- [x] **WB-109：实现原生本地视频导入。** 依赖：WB-102。
  - 完成证据：`local_video.py`、`library_import.rs`、`library/local_import.rs` 和原生选择命令实现探测、重复文件 / 硬链接识别、取消、退出回收及重新关联；真实 FFprobe 与隔离 Core 验证见 [第二批实施记录](workbench-knowledge-base-validation.md#2026-10-27本地导入与资产转录)。原生对话框人工操作及页面接入尚待阶段验收。
  - 交付：原生选择器、媒体探测、文件身份去重、入库结果和重新关联入口。
  - 验收：取消选择无副作用，重复选择不重复入库，损坏或不支持的文件明确失败；探测与持久化成功后才反馈入库成功，不默认复制大文件。

- [x] **WB-110：将转录成功结果切换到资产文档。** 依赖：WB-103、WB-106。
  - 完成证据：schema v2 和 `transcription.rs` 按文件持久化摘要，成功正文只存在资产文档；旧任务入口与新资产入口共用操作。关联 B 后旧文案仍引用 A 并明确标记来源变化，失败 / 取消不覆盖成功文案或整理稿；验证见 [第二批实施记录](workbench-knowledge-base-validation.md#2026-10-27本地导入与资产转录)。
  - 交付：资产 / 文件转录入口、旧任务入口过渡、成功版本保存和操作进度映射，移除长期正文双写。
  - 验收：保持单转录并发、关闭弹窗继续处理及现有取消回收；CPU / CUDA、失败、无语音、中断和源文件变化保留正确语义，不覆盖整理稿。

- [x] **WB-111：实现中文检索和组合筛选。** 依赖：WB-103、WB-104、WB-108。
  - 完成证据：`library/search.rs` / `search.sql` 实现参数化子串、组合筛选、文件状态与键集分页、历史版本命中摘要；中文单字及特殊字符测试通过，构造体量及实测延迟见 [第三批实施记录](workbench-knowledge-base-validation.md)。
  - 交付：标题、作者、原文、整理稿、笔记的参数化子串搜索，分类 / 标签 / 平台 / 文件状态筛选，稳定分页和命中摘要。
  - 验收：中文短词、`%` / `_` 字面搜索、归档排除、跨字段及跨分类结果正确；记录真实体量和延迟，再判断是否需要索引升级。

- [x] **WB-112：接通知识库 Tauri 命令和前端类型。** 依赖：WB-103、WB-104、WB-107、WB-109、WB-110、WB-111。
  - 完成证据：`library_api.rs`、`library_commands.rs` 和 `src/lib/library-api.ts` 接通 23 条新增命令及摘要事件，旧导入 / 转录接口保留；权限归属、修订冲突、失败不通知及原下载删除入口失效通知均有回归，桌面特性 Clippy 和 TypeScript 构建通过。完整契约见 [API 文档](workbench-knowledge-base-api.md)，页面和原生人工验收仍由后续任务完成。
  - 交付：资产、分类标签、文案、笔记、检索及文件管理的强类型调用和变更通知，沿用已有错误传播与事件机制。
  - 验收：界面通过 Rust 服务读写，错误持续可见；列表使用摘要，不随进度事件反复传输全文，不把凭据或媒体签名 URL 暴露给前端。

- [x] **WB-113：实现导航、知识库列表和分类标签交互。** 依赖：WB-002、WB-112。
  - 完成证据：`knowledge-library.tsx`、`library-organization.tsx`、`use-library.ts` 接入真实 Rust API，`main.tsx` 提供导航与工作台入口；知识库三视口与边界浏览器脚本 7 + 3 组、原后台下载 15 组及 `npm run build` 通过，见 [第四批实施记录](workbench-knowledge-base-validation.md#2026-10-27导航与知识库列表)。分类标签保存及重载使用隔离 fixture 验证；真实 Tauri 重启、原生选择器和多平台验收仍由阶段验收完成。
  - 交付：工作台 / 知识库 / 下载中心入口，资产列表、搜索筛选、分类树、标签管理、批量关联、待分类和归档恢复视图。
  - 验收：真实数据驱动，筛选和多选反馈明确；重启数据正确；空态、长标题、键盘焦点和三个规定视口可用。

- [x] **WB-114：实现资产详情、视频播放和文件状态。** 依赖：WB-113。
  - 完成证据：`library-asset-detail.tsx` / `library-video.tsx` 接入来源、资料摘要、文件版本、内置播放和重新关联；新增 `open_library_file` 复用核心归属校验。详情 6 组、真实 VP8 / H.264 浏览器解码及资源释放 7 组、原知识库 7 + 3 组、Core API 5 项、桌面 Clippy 和构建通过，见 [第五批实施记录](workbench-knowledge-base-validation.md#2026-10-27资产详情与视频播放)。真实 Tauri 播放 / 原生选择器 / 系统播放器及 macOS 尚待阶段验收；正文阅读编辑由 WB-115 实施。
  - 交付：播放器、文件版本选择、来源信息、封面、缺失状态和重新关联，以及无法内嵌播放时打开本地文件的入口。
  - 验收：横竖视频、长路径和不同编码分别验证；无法播放时如实反馈，不自动转码，文件缺失不阻止阅读资料。

- [x] **WB-115：实现原文阅读、整理稿编辑和笔记界面。** 依赖：WB-103、WB-110、WB-114。
  - 完成证据：`library-writing.tsx`、`use-library-writing.ts` 和 `library-editor-navigation.tsx` 实现版本阅读、显式保存、笔记引用、冲突恢复及未保存保护；`export_library_document` 接入原生 TXT 导出。编辑主流程 5 组、失败边界 9 组、原详情 6 组 / 视频 7 组 / 作者 6 平台三视口、Core API 6 项、旧转录 8 项、构建和桌面 Clippy 通过，见 [第六批实施记录](workbench-knowledge-base-validation.md#2026-10-27文案编辑与笔记)。实际系统剪贴板 / 导出选择器、Tauri 退出和真实 ASR 模型未进行人工验收。
  - 交付：原文 / 整理稿切换、显式保存、修订冲突反馈、笔记编辑及引用来源展示，复用现有复制和导出能力。
  - 验收：长文案滚动、全选复制、编辑未保存、失败重试和重新转录均可用；旧模型和旧文案版本正确展示，人工内容不被自动覆盖。

- [x] **WB-116：实现基础工作台。** 依赖：WB-113、WB-115。
  - 完成证据：`workbench.tsx`、`workbench.rs` 与 schema v3 接通真实摘要和访问持久化，待办由分类 / 整理稿 / 笔记派生；任务按 ID、转录按资产和文件版本准确跳转。29 项相关 Rust 测试、三视口工作台及编辑回归、构建和桌面 Clippy 已通过，见 [第七批实施记录](workbench-knowledge-base-validation.md#2026-10-27基础工作台)。Tauri 原生窗口、真实 ASR 和 macOS 人工验收未运行。
  - 交付：最近访问、待分类、待整理及运行中操作的真实入口，并补齐必要的访问时间与整理状态存储 / 计算规则。
  - 验收：最近访问在重启后保留；每个入口能到达对应资产或操作；状态由实际文案、分类和笔记确定，空工作台无虚构内容。

- [x] **WB-117：实现单篇导出、逻辑备份与空库恢复核心。** 依赖：WB-103、WB-104、WB-105、WB-108。
  - 完成证据：`library/content_export.rs`、`library/backup.rs`、`library/transfer.rs` 与 `library_transfer.rs` 实现单篇三格式、版本化目录包、完整性 / 关系校验、流式媒体复制、取消回收及空库事务恢复。39 项知识库相关 Rust 测试和桌面 Clippy 已通过，完整验证与审查见 [第八批实施记录](workbench-knowledge-base-validation.md#2026-10-27导出备份与空库恢复核心)。新格式及备份选择器 / 页面接入仍属 WB-118。
  - 交付：沿用已接入的单篇 TXT，增加 Markdown / JSON 导出、带版本的知识库逻辑包、仅资料 / 包含视频两种模式，以及包校验和事务恢复。
  - 验收：不导出应用设置或凭据；恢复保留版本与引用；已有资料时拒绝覆盖，缺媒体可恢复文本，包内路径不能越过恢复目录；失败无半成品资料。

- [x] **WB-118：接入导出、备份恢复界面。** 依赖：WB-002、WB-115、WB-117。
  - 完成证据：`library-backup-settings.tsx`、`use-library-transfer.ts`、`library_transfer_commands.rs` 和 `library_transfer_operation.rs` 接入三格式 / 笔记导出、原生文件 / 目录选择器、真实后台进度、取消与退出回收；新界面 4 组、编辑 14 组、工作台 4 组浏览器回归、41 项知识库及完整 Rust 174 项、构建和桌面 Clippy 通过。原生隔离 harness 编译成功但启动被 `0xc0000139` 阻断，真实 GUI 为 UNAVAILABLE，见 [第九批实施记录](workbench-knowledge-base-validation.md#2026-10-27导出备份恢复界面与原生命令)；阶段验收仍未勾选。
  - 交付：单篇导出和设置中的备份恢复入口、原生目录选择、范围选择、真实进度与结果，以及升级前备份和逻辑包的用途说明。
  - 验收：明确默认不含大视频，取消选择不执行；错误与成功路径可核对，恢复后列表立即与数据库一致。

- [ ] **WB-119：完成 P1A 集成与阶段验收。** 依赖：WB-108、WB-109、WB-110、WB-114、WB-115、WB-116、WB-118。
  - [x] Windows：排查 `0xc0000139`，修正 Cargo example 的 manifest / 图标资源链接；隔离原生窗口完成导入、播放、编辑、分类标签、检索、两种备份、空库恢复与升级重启验证。
  - [x] 集成回归：全量 Rust 174 项通过、2 项既有在线测试忽略，新增贯穿学习 / 历史清理 / 仅资料恢复的集成测试通过；Python 463 项、构建、桌面 Clippy 和 8 份知识库浏览器回归通过。详见 [第十批实施记录](workbench-knowledge-base-validation.md#2026-10-27p1a-集成与-windows-原生验收)。
  - [ ] macOS Intel 原生验收：`UNAVAILABLE`，无可用机器，待补证据。
  - [x] macOS Apple Silicon：旧库升级、导入 / 播放、整理稿 / 笔记、两种备份、空库恢复及恢复后范围播放通过；原生分类标签保存 / 重启、中文组合检索、键盘和 1440×900 / 1280×800 / 1024×680 窗口验证通过，并修复返回列表焦点。见[第 31 批记录](workbench-knowledge-base-validation.md#第-31-批macos-原生组合验收与取消回收修复)。
  - 交付：更新实施验证记录及相关技术 / 设计文档，覆盖升级和空库两种启动方式、完整学习流程、备份恢复，以及现有下载 / 转录回归。
  - 验收：方案第 11 节 P1A 条目全部有证据；类型 / 构建与受影响 Rust / Python 检查通过，浏览器和三种原生平台分别记录，必需未测项保持待验收。

## P1B：MCP 只读接入

技术依据：方案第 4、8、11 节。先提供资料读取，AI 总结暂由用户手动保存。

- [ ] **WB-201：验证官方 SDK 与客户端兼容性。** 依赖：WB-001。
  - [x] 技术选型：固定官方 `rmcp 3.8.0`、Apache-2.0、Rust 1.88，确认 2026-07-28 与旧 2025-11-25 两种生命周期；独立 probe 不改变生产依赖。
  - [x] Windows 验证：SDK 编译 / Clippy、官方 TypeScript SDK 1.32.1 握手与调用、现代协议固定报文、Unicode 与 EOF，以及真实 Codex CLI 0.162.1 工具发现通过，见 [协议与选型验证](mcp-sdk-validation.md)。
  - [ ] macOS Intel 构建与客户端连接：`UNAVAILABLE`，无可用机器，保留待办。
  - [x] macOS Apple Silicon：rmcp probe 构建、官方 TS SDK 双生命周期 / Unicode / EOF 和真实 Codex CLI 0.161.0 工具发现通过。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 状态：技术选型、Windows 与 Apple Silicon 验证完成；Intel 无可用机器，保留 UNAVAILABLE。生产接入的分步进度见 WB-202～WB-208。
  - 交付：官方 Rust MCP SDK 的版本、许可证、所支持协议及最小 stdio 验证结论，确定至少一个目标本机 AI 客户端和版本。
  - 验收：记录官方资料和可运行结果，确认 Windows / macOS 构建与客户端握手；不自行实现完整协议，不将最小样例当生产接入完成。

- [ ] **WB-202：实现应用核心的本机 IPC 服务。** 依赖：WB-119、WB-201。
  - [x] Core 服务与 Windows 命名管道：默认关闭、当前用户受保护 DACL、明确只读白名单、实际状态与启停命令、关闭会话及应用退出回收，见 [IPC 契约](mcp-local-ipc.md)。
  - [x] Windows 验证：实际管道 / DACL、独立进程通信、实时 Core 数据、非法请求、隔离、重复启停和关闭回收通过，命令与证据见 [实施记录](workbench-knowledge-base-validation.md)。
  - [ ] macOS Intel 编译与 Unix socket 原生验收：实现已落盘，`UNAVAILABLE`，无可用机器。
  - [x] macOS Apple Silicon：Unix socket 权限、长中文路径 / profile 隔离、非法请求、重复启停、跨进程 Core 读取与关闭回收通过。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 状态：Windows 与 Apple Silicon 子项完成；Intel 无可用机器（UNAVAILABLE），总项保持未勾选。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 交付：Windows 命名管道和 macOS Unix domain socket，当前用户访问限制、明确操作白名单和应用退出时的关闭处理。
  - 验收：连接只到正在运行的同一应用核心；断连、关闭和非法请求正确失败；不开放 HTTP，不新增直接写库或独立调度器。

- [ ] **WB-203：实现 stdio MCP 适配进程。** 依赖：WB-202。
  - [x] 独立入口：可选 `mcp` feature 与 `yingqu-mcp --endpoint <系统端点>`，固定 rmcp 3.8.0 SDK、stdout 协议专用及固定错误码诊断，见 [适配进程契约](mcp-stdio-adapter.md)。
  - [x] IPC 映射：同一 Core 的分类 / 标签工具、持久连接与响应校验、空闲断连监督、5 秒交换超时；不隐式启动应用、重连或直接开库。
  - [x] Windows 验证：官方 TS SDK 握手 / 调用、现代 discovery、EOF、服务关闭、应用退出、stdin 保持打开时进程回收，以及关闭后重新开启的新进程连接通过，见 [实施记录](workbench-knowledge-base-validation.md)。
  - [ ] macOS Intel 构建、握手及原生进程回收：`UNAVAILABLE`，无可用机器，保留待办。
  - [x] macOS Apple Silicon：本机构建、真实 stdio / Unix socket 握手、EOF、服务 / Core 断连及 helper 回收通过。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 状态：Windows 与 Apple Silicon 子项完成；Intel 无可用机器（UNAVAILABLE），总项保持未勾选。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 交付：独立启动入口、SDK 握手、MCP 到本机 IPC 的映射及诊断日志；stdout 只承载协议。
  - 验收：服务关闭或应用未启动明确不可用；进程正常退出和断连可回收，不隐式启动主应用，不泄露本地诊断中的敏感内容。

- [ ] **WB-204：实现 P1B 只读工具。** 依赖：WB-203、WB-111、WB-112。
  - [x] 七项工具：公开输入 / 输出 Schema、只读注解、同一 Core 领域服务和 IPC 映射，见 [只读工具契约](mcp-read-tools.md)。
  - [x] 读取语义：分页默认 20 / 最大 100，正文默认 4000 / 最大 16000 Unicode 标量，文档版本 / 笔记修订及精确来源范围，限长且分页的引用摘录。
  - [x] Windows 验证：Core / 适配器测试与官方 TS 客户端真实 stdio 调用覆盖空结果、未知 / 跨资产 ID、超限、越界、游标、旧版本、修订冲突、公开字段及错误后继续读取，见 [实施记录](workbench-knowledge-base-validation.md)。
  - [ ] macOS Intel 构建及七项工具原生验收：`UNAVAILABLE`，无可用机器，保留待办。
  - [x] macOS Apple Silicon：七项只读工具、筛选 / 分页 / Unicode、版本 / 修订与错误边界，以及安装后真实客户端读取通过。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 状态：Windows 与 Apple Silicon 子项完成；Intel 无可用机器（UNAVAILABLE），总项保持未勾选。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 交付：`search_assets`、`get_asset`、`read_transcript`、`list_categories`、`list_tags`、`list_notes`、`read_note` 的 schema 和领域服务适配。
  - 验收：工具结果与桌面数据一致，支持分页、版本和引用；未知 ID、空结果、超限及越界有契约测试；Unicode 位置及返回大小符合方案。

- [ ] **WB-205：实现受限 Resources 及读取边界检查。** 依赖：WB-204。
  - [x] 资源入口：资产 / 文档 / 笔记稳定 URI、三个模板、受限目录和 JSON 文本读取，见 [Resources 契约](mcp-resources.md)。
  - [x] 共用边界：直接复用现有类型化工具 / IPC / Core 读取，保留分页、归属、版本 / 修订、Unicode 和大小限制，不新增文件或读库通道。
  - [x] Windows 验证：资源 / 工具深比较、同域失败、非法 URI、合成凭据 / 路径哨兵检查、Tools-only 七工具读取及原有生命周期回归通过，见 [实施记录](workbench-knowledge-base-validation.md)。
  - [ ] macOS Intel 构建、资源读取及边界原生验收：`UNAVAILABLE`，无可用机器，保留待办。
  - [x] macOS Apple Silicon：资源 / 工具范围一致、URI / 路径 / 合成凭据边界及 Tools-only 读取通过。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 状态：Windows 与 Apple Silicon 子项完成；Intel 无可用机器（UNAVAILABLE），总项保持未勾选。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 交付：稳定资产 / 文档 / 笔记 URI、与工具一致的访问范围，以及不依赖 Resources 的工具读取路径。
  - 验收：Resources 不能绕过范围限制；工具及资源不返回设置、凭据、签名 URL 或任意绝对路径；不支持 Resources 的客户端仍可完成读取。

- [ ] **WB-206：实现 MCP 开关和连接配置界面。** 依赖：WB-002、WB-203。
  - 交付：默认关闭、启停状态、客户端启动配置复制和数据使用范围说明，主应用关闭后状态可理解。
  - 验收：未开启不能连接，开启后按真实安装路径生成可用配置，含空格的路径正确；不将云端客户端写成本机 stdio 可用。
  - [x] 设置页 MCP 分类、默认关闭、真实启停 / 异常 / 读取失败状态与重试、启停中反馈及数据范围说明。
  - [x] Rust 按当前主程序同目录生成 Codex TOML / 通用 JSON；缺少适配程序明确禁用，复制时复核状态并使用原生剪贴板。
  - [x] Windows：三个视口与键盘交互、失败恢复、旧读取丢弃、原生复制、中文空格目录、SDK 启动读取及 Codex CLI 配置发现通过；关闭后旧配置不能连接。
  - [ ] macOS Intel：设置界面、剪贴板、实际程序目录与启停连接验收（当前无设备，UNAVAILABLE）。
  - [x] macOS Apple Silicon：原生设置默认关闭、真实启停、JSON 原生复制 / 粘贴核对、TOML / JSON 一致，以及 SDK / Codex 实际路径连接通过。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 状态：Windows 与 Apple Silicon 子项完成；Intel 无可用机器（UNAVAILABLE），总项保持未勾选。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。

- [ ] **WB-207：将适配进程纳入安装包并验证连接。** 依赖：WB-205、WB-206。
  - 交付：构建、分发、许可和平台打包集成，从安装后的配置启动适配进程并连接主应用的记录。
  - 验收：Windows、macOS Intel / Apple Silicon 分开记录；用户不依赖开发目录或额外安装开发运行时，签名 / 应用更新沿用现有分发要求。
  - [x] 显式 desktop,mcp 构建与 CI / Release 接入；保留完整 8 包 / 13 文件许可，检查缺失二进制、安装内容和许可；沿用 Windows 无签名、macOS ad-hoc 及原有整包更新机制。
  - [x] Windows x64 NSIS 实际安装到中文空格目录；安装后内嵌界面复制配置、系统 PATH 下 SDK 启动读取、Codex CLI 0.162.1 七工具发现、关闭拒绝、重启默认关闭 / 重连及卸载通过。
  - [x] 打包检查回归 7 项、许可一致性、生产构建与 Clippy 通过；验证全新 cache 下的检查顺序，修复并验证 CI 签名错误不能被后续成功检查覆盖。
  - [ ] macOS Intel：DMG / app 安装、辅助程序签名、实际路径和客户端连接验收（当前无设备，UNAVAILABLE）。
  - [x] macOS Apple Silicon：ad-hoc app / DMG 构建、主程序和 helper 深层签名、完整许可、DMG 复制到中文空格目录及安装后连接通过；非发布 / Gatekeeper 下载验收。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 状态：Windows 与 Apple Silicon 子项完成；Intel 无可用机器（UNAVAILABLE），总项保持未勾选。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。

- [ ] **WB-208：完成 MCP 真实客户端和 P1B 验收。** 依赖：WB-207。
  - 交付：一个明确名称与版本的真实客户端完成筛选、读取、多视频对比和来源引用，记录服务关闭、断连、重连及返回范围的行为。
  - 验收：自动协议、真实客户端、总结内容质量分别记录；结果可回到正确资产和文档版本；确认只读阶段不能修改资料，不把本地转录误称为外部模型本地推理。
  - [x] Windows 安装后的真实 Codex CLI 0.162.1 完成七项读取、组合筛选与分页、Unicode 及错误边界；模型 gpt-6.1-sol 自行检索两条合成视频并完成对比。
  - [x] 四处原句逐项匹配成功工具响应，原生界面核对资产、A v2 / B v1 和笔记 r1；总结质量单独审阅，明确未观看视频帧、没有时间戳或效果数据。
  - [x] 协议与模型阶段 23 张逻辑表无变化；服务关闭、故障断连、重新开启、应用重启默认关闭及新会话重连通过，客户端自然退出且无本轮 helper 残留。
  - [x] 验收脚本回归 Node 15 项、Python 2 项，Core 196 passed / 2 ignored，构建及 Windows 实际安装通过；未改产品行为或日常客户端配置。
  - [ ] macOS Intel：安装后真实客户端、模型组合、原生来源核对及生命周期验收（当前无设备，UNAVAILABLE）。
  - [x] macOS Apple Silicon：安装后 Codex CLI 0.161.0 / gpt-6.1-sol 完成组合筛选、对比、四处引用与原生来源核对；只读 SDK 快照、服务断连 / 重连及重启默认关闭通过，模型内容另经 Codex 审阅。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 状态：Windows 与 Apple Silicon 子项完成；Intel 无可用机器（UNAVAILABLE），总项保持未勾选。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。

## P2：拆解学习与创作复用

技术依据：方案第 5.4、7.1、8、11 节。时间戳和多模态能力先验证后接入。

- [ ] **WB-301：验证真实文案时间信息来源。** 依赖：WB-119。
  - 交付：检查现有 VAD / ASR 输出，使用长短、含停顿、中英文样本验证分段范围，记录精度和失败条件；确需额外对齐能力时先记录选型结论。
  - 验收：决定可交付的段落级或更细精度；不按字数分摊时间，不把进度片段数当时间戳。结果不足时阻塞精确同步，保留全文能力。
  - [x] 核对固定 CPU CLI 的 `--srt` 与独立 FSMN-VAD；CUDA 暂存区间 / 逐段正文可一一对应，当前生产只保存全文，进度不是时间来源。
  - [x] Windows 实跑短中文、含停顿中英文、66 秒连续语音和约 298 秒自然口播，比较 CPU SRT、独立 VAD、真实 CUDA 裁剪采样及正文映射；输入哈希不变，文本质量限制单独记录。
  - [x] 复现音频延迟 +1500ms 时当前 WAV 丢掉延迟，候选 FFmpeg 归零方案保留准确静音采样；明确未验证的其他容器 / 原点 / 实际播放器为 WB-302 前置条件。
  - [x] 选择 `fsmn_vad / vad_segment` 语音段级范围；10ms 是网格，不是准确率。复用已有原生引擎，不安装额外对齐依赖；逐字 / 句子 / 卡点同步继续阻塞，旧文案保留无时间。
  - [ ] macOS Intel：固定 CPU CLI、VAD、媒体时钟及原生时间定位条件复验（当前无设备，UNAVAILABLE；不要求 Windows CUDA）。
  - [x] macOS Apple Silicon：短中文、含停顿中英文、66 秒连续语音、上游 8 秒样本及 11 组媒体时钟已有证据；本轮再实跑完整 628.541 秒解说与 466.782 秒具名真人朗读，分别 58 / 129 个 CPU SRT 与独立 VAD 窗口一致。完整视频经原生真实转录入库，段落 / 正文与独立 SRT 一致，427.390 / 621.890 秒跳转及重启通过。只交付语音段级范围，保留错字、跨句 / 切词和准确率未验收限制；原 Windows 约 298 秒同输入比较 NOT_RUN。见[第 39 批记录](workbench-knowledge-base-validation.md#第-39-批macos-长语音时间来源复验)。
  - 状态：Windows 与 Apple Silicon 的时间来源子项完成；Intel UNAVAILABLE，总项未勾选。未承诺逐字 / 句子 / 卡点精度或全文质量，WB-302 按既有语音段级契约实现。

- [ ] **WB-302：保存可追溯的文案段落时间。** 依赖：WB-301。
  - 交付：段落实体、文件 / 文档版本关联、时间精度来源及转录协议的必要扩展，同步调整 Rust / Python 边界。
  - 验收：时间范围有序合法、旧文案允许无时间；失败不覆盖成功版本，重转录或换文件后旧引用不串位。
  - [x] 统一为媒体呈现 PTS 零点：保留 copyts，以 aresample 填补正延迟 / 裁去零点前音频；11 组容器 / 编码 / VFR / 正负时间矩阵及 5 组 Edge 解码和 seek 通过，不宣称音画同步精度。
  - [x] CPU SRT、CUDA VAD 映射生成固定 schema v1 时间文件；Rust 校验结构、大小、窗口和正文后赋操作 ID，同秒重识别生成不同版本，进度不冒充时间。
  - [x] schema v4 的段落与不可变文案、文件 ID、Source 指纹及模型 / VAD / pipeline 关联；旧库一致备份 / 事务升级，旧文案及整理稿无自动时间。
  - [x] 非法产物、取消、无语音、源变化、段落插入中途失败和操作保存失败保留旧结果；版本、换文件、升级回滚与重试验证通过。
  - [x] 备份 writer v2 / reader v1、v2 保留时间及绑定，恢复重定位识别键保持幂等；Windows 真实 Core + 源码 CPU / CUDA + 打包 CPU 完成两版本 / 重开验证，相关回归及 Clippy / 类型构建通过。
  - [ ] macOS Intel：CPU / VAD、PTS 零点、文件绑定、恢复及打包侧进程复验（当前无设备，UNAVAILABLE）。
  - [x] macOS Apple Silicon：真实 CPU / VAD、11 组 PTS 矩阵、源码 / 打包引擎各生成两份不可变时间版本并重开绑定；原生备份恢复保留段落 / 文件 / 引用，恢复后范围播放通过。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。
  - 状态：Windows 与 Apple Silicon 子项完成；Intel 无可用机器（UNAVAILABLE），总项保持未勾选。见[第 30 批记录](workbench-knowledge-base-validation.md#第-30-批macos-apple-silicon-验收)。

- [ ] **WB-303：实现片段收藏与范围播放。** 依赖：WB-302、WB-114。
  - 交付：片段开始 / 结束标记、标题、收藏理由和文案引用，片段列表与范围播放。
  - 验收：默认仅保存原视频引用，不重复裁剪文件；无时间文案允许手动标记；缺文件、越界范围和不同文件版本准确反馈。
  - [x] 详情页内联标记起止、秒数输入、标题 / 理由、列表与删除；无时间文案可手动收藏，不生成媒体副本。
  - [x] 保存不可变文案 ID / version / 原句、文件 ID 与 Source，沿整理稿来源链检查；重识别、重新关联、变更与缺失不串位，旧文案仍可读。
  - [x] 每次准备播放核对 Source，切换到收藏所绑定文件；实测 seek 下界并使用动态 duration / seekable，拒绝头尾越界或无法完整定位的范围，不静默截短。
  - [x] 范围到点暂停；播放中范围收缩或提前结束明确失败；媒体 / 回调回收、双草稿导航和保存失败保留通过，三视口及既有阅读 / 播放回归通过。
  - [x] schema v5 一致性备份 / 事务升级及失败重试；逻辑包 writer v3、reader v1 / v2 / v3，含 / 不含媒体及历史源的二次备份恢复通过；历史片段源纳入导出保护。
  - [x] Windows 实际 Tauri UI / 命令 / SQLite / asset protocol 完成普通及正 PTS MP4 播放、错文件拒绝、缺失 / 变化拒播、空闲结束实例后重开保留验证；Core / Clippy / 类型构建及独立复审通过。
  - [ ] macOS Intel：原生播放器、seek / 动态范围、版本 / 文件、升级及恢复复验（当前无设备，UNAVAILABLE）。
  - [x] macOS Apple Silicon：真实 Tauri 普通 / 正 PTS MP4、实际 seek 下界与尾越界、文件版本切换、大小 / mtime / 缺失 / 重关联拒播、固定引用、保存失败与双草稿、三窗口 / 键盘 / 重启及两种备份恢复通过；原生 WKWebView 受控事件夹具补充范围收缩 / 提前结束，arm64 Core 补充迁移 / 事务 / 二次恢复。证据层级与限制见[第 36 批记录](workbench-knowledge-base-validation.md#第-36-批macos-片段收藏与范围播放原生验收)。
  - 状态：Windows 与 macOS Apple Silicon 限定完成，总项等待 Intel，主任务仍 21 / 49；非逐帧剪辑或逐字同步。见[片段契约](asset-clips.md)及[第二十一批记录](workbench-knowledge-base-validation.md#第二十一批wb-303-片段收藏与范围播放)。

- [ ] **WB-304：实现关键帧提取和资源管理。** 依赖：WB-303。
  - 状态：Windows 与 macOS Apple Silicon 子项完成；Intel 无可用机器，继续 UNAVAILABLE，总项保持未勾选。
  - [x] 指定位置与片段均匀采样，保存请求 / 实际 PTS、文件与片段来源；横竖 / SAR / 黑帧可追溯。
  - [x] PNG 存储、按归属读取、预览 / 删除、源缺失与变化状态，以及 schema v6 / 备份 v4 恢复。
  - [x] Windows 实际 FFmpeg、Tauri GUI、缺失 / 无效范围、事务失败、取消 / 退出进程回收及回归验证。
  - [ ] macOS Intel 原生提取、资源 / 文件状态、取消回收与备份恢复验收。
  - [x] macOS Apple Silicon：原生横屏 / 竖屏黑帧 / SAR / 正 PTS、3 / 5 / 9 帧与窄区间整批失败、分页 / 删除、源变化 / 缺失 / 重关联、真实引擎与 FFmpeg 取消 / 退出回收、三窗口键盘 / 重启及两种备份恢复通过；修复关闭预览后的焦点恢复。详见[第 37 批记录](workbench-knowledge-base-validation.md#第-37-批macos-关键帧原生验收与预览焦点修复)。
  - 证据：[关键帧契约](asset-keyframes.md)、[第二十二批记录](workbench-knowledge-base-validation.md#第二十二批wb-304-关键帧提取与资源管理)。
  - 交付：复用 FFmpeg 的指定位置抽帧、采样范围、关键帧存储与展示，并沿用取消和子进程回收机制。
  - 验收：横竖、黑帧、无效时间和缺少依赖可诊断；失败不影响原视频；资源有真实时间与采样信息。

- [ ] **WB-305：实现视频与文案同步阅读。** 依赖：WB-302、WB-303。
  - 状态：Windows 与 macOS Apple Silicon 子项完成；Intel 无可用机器，继续 UNAVAILABLE，总项保持未勾选。
  - [x] 有时间原文的段落跳转、当前段高亮、静音间隙处理及自动滚动开关。
  - [x] 局部 Space / 方向键 / Enter、可访问段落及连续段落片段预填，保留固定版本 / 文件引用。
  - [x] Windows 三视口与原生短 / 长样本，正 PTS、无效范围、来源变化、编辑键盘隔离及既有流程回归。
  - [ ] macOS Intel 原生同步、键盘、来源与片段选取验收。
  - [x] macOS Apple Silicon：既有真实 CPU 短样本 / 66 秒结果复验，300 秒 / 150 段合成 UI 夹具的远端定位 / 跟随 / 自由滚动，局部键盘、静音间隙、连续选段与固定版本、正 PTS / 越界、错文件 / 源变化 / 缺失 / 重关联、三窗口与重启通过；不计作 WB-301 长自然语音或 ASR 精度验收。证据分层见[第 38 批记录](workbench-knowledge-base-validation.md#第-38-批macos-同步阅读原生验收)。
  - 证据：[同步阅读契约](transcript-sync-reading.md)、[第二十三批记录](workbench-knowledge-base-validation.md#第二十三批wb-305-视频与文案同步阅读)。
  - 交付：点击段落跳转、播放跟随、自动滚动开关、键盘播放及片段选取。
  - 验收：按 WB-301 确认的精度验证长短样本；禁用跟随后用户可自由阅读，无时间文案保留全文阅读入口。

- [ ] **WB-306：实现结构化文案和制作手法拆解。** 依赖：WB-103、WB-303、WB-304。
  - 状态：Windows 与 Apple Silicon 子项完成；Intel 无可用机器，继续 UNAVAILABLE，总项保持未勾选。
  - [x] 六字段文案模板、四维度时间范围观察、片段 / 关键帧和固定文案版本引用。
  - [x] 人工 / AI 创建来源与模型、AI 人工修订标识和观察依据；静图不能作为声音依据，未读取不冒充删除。
  - [x] Windows 三视口、真实 Tauri / SQLite / PNG / 播放、空闲重开、冲突恢复、schema v7 / 备份 v5 与既有读取回归。
  - [ ] macOS Intel 原生表单、来源、引用 / 播放及迁移恢复验收。
  - [x] macOS Apple Silicon：原生六字段与四维度表单、合成 AI 手动录入 / 人工修订来源、固定文案 / 片段 / PNG 引用、范围播放、三窗口 / 键盘与重启通过；两种原生备份在独立空库恢复后，结构化笔记和引用一致，PNG / 5 个媒体哈希一致，缺媒体播放禁用。10 项拆解核心回归通过，迁移 / 冲突等核心边界与原生结果分别记录。见[第 32 批记录](workbench-knowledge-base-validation.md#第-32-批macos-结构化拆解原生验收)。
  - 证据：[拆解契约](structured-analysis.md)、[第二十四批记录](workbench-knowledge-base-validation.md#第二十四批wb-306-结构化文案和制作手法拆解)。
  - 交付：开头、观点、论据、转折、结尾及可借鉴点模板；画面、字幕、转场、声音的时间范围笔记和引用展示。
  - 验收：人工 / AI 来源清楚；文案拆解与视觉观察分别记录，仅看截图不能声明听过配乐或看过全部连续运动。

- [ ] **WB-307：实现跨视频对比工作区。** 依赖：WB-111、WB-306。
  - 状态：Windows 与 Apple Silicon 实现和验证完成；Intel 无可用机器，继续 UNAVAILABLE，总项保持未勾选。
  - [x] 2～4 资产多选、同维度对照、缺失显示未知及每条结论独立选择固定来源。
  - [x] 带来源比较笔记、原文 / 拆解 / 片段打开、来源更新拒绝、结论编辑保留旧引用和草稿保护。
  - [x] Windows 三视口 / 键盘 / 原生 Tauri / SQLite / 空闲重开，schema v8 / 备份 v6 及引用隔离回归。
  - [ ] macOS Intel 原生对照、来源、播放、键盘、迁移与恢复验收。
  - [x] macOS Apple Silicon：原生 2 / 4 视频对照、未知维度、逐条结论固定来源、版本切换 / 人工修订后历史摘录、原文 / 拆解 / 范围播放、草稿保护、三窗口 / 键盘及重启通过；修复窄窗口键盘聚焦未完整滚入视口。含 / 不含视频的原生备份在独立空库恢复后，比较和笔记逐行一致，5 个媒体及 PNG 哈希一致；6 项比较核心回归通过。见[第 33 批记录](workbench-knowledge-base-validation.md#第-33-批macos-跨视频对比原生验收)。
  - 证据：[比较契约](video-comparison.md)、[第二十五批记录](workbench-knowledge-base-validation.md#第二十五批wb-307-跨视频对比工作区)。
  - 交付：选择多个资产，对照开头、文案结构、片段和拆解，保存带来源的比较笔记。
  - 验收：每项结论能打开对应资料，不把不同视频的引用混用；缺失维度显示未知。

- [ ] **WB-308：实现研究专题。** 依赖：WB-306。
  - 状态：Windows 与 Apple Silicon 实现和验证完成；Intel 无可用机器，继续 UNAVAILABLE，总项保持未勾选。
  - [x] 专题创建 / 编辑，组合视频、片段、笔记，资料搜索和成员增删 / 上下排序。
  - [x] 成员移除仅解除关系，缺失 / 归档 / 原片段来源变化可见，原文 / 片段 / 笔记与视频返回导航。
  - [x] Windows 三视口 / 键盘 / Tauri / SQLite / 真范围播放 / 空闲重开；schema v9 / 备份 v7 两轮含 / 不含媒体恢复关系与顺序。
  - [ ] macOS Intel 原生专题、成员 / 来源、键盘 / 播放和迁移 / 恢复验收。
  - [x] macOS Apple Silicon：原生专题创建 / 编辑、三类成员增删 / 去重 / 键盘排序、来源视频往返、当前笔记 / 固定原文 / 比较历史摘录、范围播放、文件缺失、三窗口和重启通过；含 / 不含视频的原生备份在独立空库恢复，专题与成员顺序逐行一致，5 个媒体和 PNG 哈希一致。7 项专题核心回归覆盖 CAS、归档 / 引用缺失、重关联和迁移等边界。见[第 34 批记录](workbench-knowledge-base-validation.md#第-34-批macos-研究专题原生验收)。
  - 证据：[专题契约](research-topics.md)、[第二十六批记录](workbench-knowledge-base-validation.md#第二十六批wb-308-研究专题)。
  - 交付：专题及成员关系、增删和排序，可组合视频、片段、笔记；在原备份格式中增加版本化支持。
  - 验收：跨分类引用不复制资料，移除成员不删除来源；缺失或归档引用可见，备份恢复保留关系。

- [ ] **WB-309：实现自己的选题和草稿。** 依赖：WB-308。
  - 状态：Windows 与 Apple Silicon 实现和验证完成；Intel 无可用机器，继续 UNAVAILABLE，总项保持未勾选。
  - [x] 选题列表、独立观点 / 草稿、视频 / 片段 / 专题参考和三个灵感入口，工作台近期入口。
  - [x] 正文 / 引用同事务 CAS，保存失败保留、完整冲突副本、提交后回读失败防重复、来源往返及缺失保留。
  - [x] Windows 三视口 / 键盘 / Tauri / SQLite / 真实范围播放 / 空闲重开；schema v10 / 备份 v8 两轮含 / 不含媒体恢复。
  - [ ] macOS Intel 原生选题编辑、来源 / 键盘 / 播放、迁移 / 恢复及重启验收。
  - [x] macOS Apple Silicon：原生三个灵感入口、独立观点 / 正文与三类参考、去重 / 移除保护、专题成员 / 视频往返、范围播放、工作台、三窗口 / 键盘和重启通过；独立生产领域写入造成真实 CAS 冲突，原生保留完整输入并生成可复制副本。两种原生备份在独立空库恢复后，选题 / 参考完整一致，媒体与 PNG 哈希一致。8 项核心回归通过。见[第 35 批记录](workbench-knowledge-base-validation.md#第-35-批macos-选题与草稿原生验收)。
  - 证据：[选题契约](ideas-and-drafts.md)、[第二十七批记录](workbench-knowledge-base-validation.md#第二十七批wb-309-自己的选题和草稿)。
  - 交付：选题列表、观点与草稿编辑、参考视频 / 片段 / 专题关联和工作台入口，扩展备份恢复。
  - 验收：自己的正文和参考原文分开保存，保存与冲突规则一致；可从灵感参考进入草稿并回到来源，重启和恢复后关系正确。

- [ ] **WB-310：开放 MCP 笔记、分类标签和草稿写入。** 依赖：WB-208、WB-306、WB-309。
  - 交付：写入开关，`create_note`、`create_category`、`add_asset_categories`、`add_asset_tags`、`create_topic`、`create_idea`，调用同一领域服务。
  - 验收：默认仍只读；写回保留 AI 来源和引用，不覆盖人工正文或原文；重复增加关系无副作用，文件删除及设置修改不在开放工具内。
  - 状态：Windows 与 Apple Silicon 实现与验证完成，详见[受控写入契约](mcp-write-tools.md)及[第 28 批记录](workbench-knowledge-base-validation.md#第-28-批wb-310-受控-mcp-写入)。schema11 / writer9 / reader1～9；未发布，总项因 Intel 待验收保持未勾选。
    - [x] 六项工具、独立写授权及 Core 白名单；服务关闭与应用重启清除授权，不能从 MCP 自行授权。
    - [x] AI 笔记 / 专题 / 草稿保留模型和精确文案引用；人工正文与原文不被覆盖；分类标签追加幂等、批次错误整体回滚；迁移与备份恢复保留来源。
    - [x] 官方 MCP SDK 真实 stdio / Windows 命名管道、原生开关与配置复制、三个视口 / 键盘 / 长引用及带授权重启持久化验证通过。
    - [ ] macOS Intel 编译、原生授权 / 连接及来源恢复验收（NOT_RUN，当前无环境）。
    - [x] macOS Apple Silicon：真实 stdio / Unix socket 六工具、原生默认只读 / 授权 / 撤销 / 服务和重启复位通过；AI 来源与固定版本引用、人工资料保留、追加幂等 / 错误批次回滚、原生备份和空库恢复通过。仅使用合成资料和显式合成模型名。见[第 31 批记录](workbench-knowledge-base-validation.md#第-31-批macos-原生组合验收与取消回收修复)。

- [ ] **WB-311：开放 MCP 转录长操作。** 依赖：WB-208、WB-110。
  - 交付：`start_transcription`、`get_operation`、`cancel_operation`，返回真实操作 ID、状态和取消结果。
  - 验收：保持单转录并发；重复启动、客户端断连、调用超时和取消有确定结果，状态不等于进程实际回收时不返回取消完成。
  - 状态：Windows 与 Apple Silicon 实现与验证完成，见[长操作契约](mcp-transcription-operations.md)及[第 29 批记录](workbench-knowledge-base-validation.md#第-29-批wb-311-mcp-转录长操作)。schema11 / 备份9不变；因 Intel 待验收，总项保持未勾选，未发布。
    - [x] 三工具及强类型 DTO / 错误、真实操作 ID、开始 / 取消写授权和查询只读；源文件按资产登记校验，不暴露路径或正文。
    - [x] 同文件运行复用 ID、全应用单转录、缓存与强制重试、新旧 ID 不混用；断连 / 已提交回执超时 / 服务重连，以及 pending取消到实际PID / 暂存回收验证通过。
    - [x] Windows 官方 SDK / 真实 stdio和命名管道、原生Tauri控制、终态与桌面一致、原文保留、重启后精确ID / 状态保留，浏览器三视口和原生授权说明通过。
    - [ ] macOS Intel 编译、原生socket / 长操作 / 进程回收验收（NOT_RUN，当前无环境）。
    - [x] macOS Apple Silicon：真实 SDK / stdio / socket 与打包 CPU ASR、原生授权 / 服务重连、同文件 ID 复用 / 跨文件占用、客户端断连、取消 pending 到实际引擎 / 子进程 / 暂存回收、完成缓存和精确终态重启保留通过。修复 Unix 终态早于整个进程组退出的问题；已提交回执超时的确定性用例仍由第 30 批协议夹具验证。见[第 31 批记录](workbench-knowledge-base-validation.md#第-31-批macos-原生组合验收与取消回收修复)。

- [ ] **WB-312：开放片段、多模态读取和分析模板。** 依赖：WB-208、WB-304、WB-307、WB-310。
  - 交付：`get_clip`、`get_keyframes` 及文案拆解 / 同题对比 / 制作观察 Prompts，记录真实客户端对图片、视频、音频的支持范围。
  - 验收：客户端能实际消费所提供媒体；不支持的媒体明确反馈；分析记录采样与时间范围，Tools 流程不依赖客户端支持 Prompts。
  - [x] 两个只读工具沿同一 Core 返回片段固定文案引用、已有 PNG 及原始 PTS / 采样范围，分页、归属和来源状态保留；512 KiB 图片预算及不支持的媒体明确反馈，不隐式抽帧或返回路径。
  - [x] 三个可选 Prompts 可发现 / 获取，强调事实、观察、建议和未观察项；只用 Tools 的流程通过，schema11 / backup9 不变。
  - [x] macOS Apple Silicon：官方 TS SDK 实际解码 PNG，原生隔离 Tauri / stdio / Unix socket 与 Codex CLI 0.161.0 / gpt-6.1-sol 实际消费两张合成图片，颜色、PTS / 采样范围和固定 v1 引用一致；启停 / 退出 / 重启回收通过。仅使用本轮明确获准的合成图片和元数据，不计作真实作品制作分析质量验收。
  - [ ] Windows：当前新增读取与模板的原生编译、客户端媒体验收 NOT_RUN。
  - [ ] macOS Intel：当前无设备，UNAVAILABLE。
  - 状态：实现与 Apple Silicon 子项完成；跨平台总项保持未勾选，正式安装包本轮未重做。契约见[片段、图片与模板](mcp-media-tools.md)，证据见[第 40 批记录](workbench-knowledge-base-validation.md#第-40-批wb-312-片段图片与分析模板)。

- [ ] **WB-313：完成 P2 学习到创作的完整验收。** 依赖：WB-305、WB-307、WB-308、WB-309、WB-310、WB-311、WB-312。
  - 交付：真实视频完成“收藏片段 → 文案 / 画面拆解 → 多视频比较 → 新选题 → AI 写回 → 来源核对”，同步技术、设计、备份格式及验证文档。
  - 验收：时间、版本、来源与文件引用准确；新增资料备份可恢复；原生播放、真实多模态客户端、文本质量和人工视听评价分别记录，未测能力不宣称通过。
  - [x] macOS Apple Silicon：实际视频的片段 / CPU 文案 / 拆解 / 比较 / 原创选题 / 受控 AI 写回 / 来源核对与最终两种原生备份恢复通过；真实客户端实际接收六张预览，人工评价限两段各 8 秒。
  - [ ] Windows：本项真实组合验收 NOT_RUN。
  - [ ] macOS Intel：无可用设备，UNAVAILABLE。
  - 状态：Apple Silicon 子项完成；全平台主项不勾选，正式安装包本批未重做；第41批时主清单为21 / 49。证据见[第 41 批记录](workbench-knowledge-base-validation.md#第-41-批wb-313-真实视频学习到创作组合验收)。

## P3：使用后评估项

这些任务只产出需求证据、取舍和后续实施方案；勾选表示评估完成，不表示功能已开发，也不自动启动对应实现。

- [x] **WB-401：收集真实使用问题并确定优先级。** 依赖：WB-313。
  - 交付：记录找回参考、整理耗时、片段复用和形成选题中的具体问题，按频率、影响和实施成本排序。
  - 验收：每个建议对应真实场景和可观察目标，不用收藏总量替代实际使用效果。
  - 完成证据：2026-10-29 [真实使用评估](workbench-real-use-assessment.md)及[第42批记录](workbench-knowledge-base-validation.md#第-42-批wb-401-真实使用问题与优先级评估)。用户明确首要阻力为“看过参考后，仍难提炼自己的选题和观点”；当前原生四步流程另确认命中跳转、笔记辨认和比较转接摩擦，分别记录小样本次数、影响、成本及可观察目标。
  - 限制：本机真实资料上的执行助手观察及1位用户反馈，不是多用户 / 长期频率或省时统计；只完成评估，不自动开发建议或启动WB-402～WB-406。

- [ ] **WB-402：评估全文索引、语义搜索和相似片段。** 依赖：WB-401。
  - 交付：基于实际数据的延迟和命中差异，比较现有查询、中文全文索引及嵌入检索的收益和资源成本。
  - 验收：明确采用、延后或不做的结论；确需依赖时记录成熟方案、许可与维护成本，不默认引入向量库。

- [ ] **WB-403：评估作者研究和智能视图。** 依赖：WB-401。
  - 交付：围绕库内资料设计作者维度和保存筛选条件，说明缺失字段及数量的真实来源。
  - 验收：不将库内作者研究扩展为平台自动采集，筛选视图不重复保存资产。

- [ ] **WB-404：评估制作资料导出和外部工具连接。** 依赖：WB-401。
  - 交付：明确目标工具、可消费格式、参考片段 / 文案 / 分镜映射及缺失能力，提出最小导出契约。
  - 验收：用目标工具实际读取样例验证，区分普通导出与第三方接口集成，不默认建设完整编辑器。

- [ ] **WB-405：评估作品发布后的人工复盘。** 依赖：WB-401。
  - 交付：自己的作品与原选题、参考资料关联，以及人工录入表现与学习结论的字段和流程。
  - 验收：区分用户录入、平台事实和推测，不把数据相关性直接视为制作手法的效果证明，不增加自动发布。

- [ ] **WB-406：分别评估远程 MCP 与自动分析。** 依赖：WB-401。
  - 交付：分别记录远程客户端需求与连接授权方案，以及自动分析的触发、模型、费用、数据范围、取消和结果保存约定。
  - 验收：两项独立决定是否立项；没有明确需求和授权时不增加公网入口、自动上传或定时模型调用。

## 实施记录要求

每项完成后的证据放在该任务下或链接到已有专项验证记录，至少包含：改动文件 / 提交、受影响行为、实际检查和未测限制。阶段完成项还需给出平台及客户端矩阵。

推荐记录格式：

```text
状态：进行中 / 阻塞 / 完成
交付：文件或提交链接
验证：实际命令、环境及结果
限制：NOT_RUN / UNAVAILABLE 的具体原因；没有则省略
下一步：未完成时需要执行的具体动作
```

文档检查使用 `git diff --check` 并核对新增文件、相对链接、任务编号和依赖。实现检查优先查 [开发文档](development.md)、`package.json` 与 CI；前端构建当前为 `npm run build`，核心检查及平台打包命令以执行时配置为准。此清单不预先宣称任何功能验证通过。
