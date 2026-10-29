# 轻量素材库与 MCP 视听分析验收

日期：2026-10-29。目标分支 `master`，起点 `c57d10d6017033e8aea2e504dc022b3d5c46ebfc`。源码实施 M01–M05 完成；自动化集成已通过。完整产品验收仍为 **UNABLE_TO_VERIFY**：Windows 原生完整交互尚未完成，macOS 当前差异无可用登记工作区，当前 Codex 模型不能消费音频，安装包本轮未制作。

## 工作区与数据边界

开始时已有六个文件改动：`DESIGN.md`、`src/main.tsx`、`src/style.css`、`src/components/knowledge-library.tsx`、两个 knowledge-library 浏览器测试。先保存基线补丁再衔接本次改造，未提交、推送或发布，未操作日常资料库。未修改 schema v11、逻辑备份包 v9、模型版本或依赖锁文件。

数据库、迁移、恢复及媒体测试使用临时 SQLite、合成媒体和 `.local/` 内带标记资料目录。真实 ASR 使用复制到独立目录的现有模型，不在日常模型或资料库中试验。Windows 原生编译目录采用独立 Tauri identifier。

## 源码职责与契约

| 模块 | 实际改动 |
| --- | --- |
| `library/assets.rs`、`local_import.rs`、`library_api.rs`、`library_commands.rs`、`desktop.rs` | 新下载默认未收录；身份与文件关联继续自动登记；收藏切换幂等；重复下载不重置收藏；手动导入主动收录；任务批量状态及设置接口 |
| `library/search.rs` / `.sql`、`mcp_read` | 列表分类、标签和文案摘要；MCP 收录范围；隐藏来源笔记在排序分页前排除；历史比较来源也校验 |
| `main.tsx`、`knowledge-library.tsx`、`use-task-collection.ts`、`task-more-menu.tsx` | 五项导航、卡片心形收藏与取消、列表收藏后打开详情、操作菜单移出、搜索/筛选/批量组织/导入；任务状态按最多 10,000 项分批查询 |
| `library-asset-detail.tsx`、`library-writing.tsx`、`use-library-writing.ts`、`library-keyframes.tsx` | 右侧仅原始文案与 AI 总结；原始文案可人工修改，总结可 Markdown 编辑预览；隐藏版本、关键帧及笔记摘要，分类标签在视频下方；保留历史数据、精确搜索跳转和未保存保护 |
| `keyframes.rs`、`library/keyframes.rs`、`mcp_operations*` | Range 3/5/9 帧、默认 5；复用唯一 worker、来源指纹、PNG 和原始 PTS；操作 ID 绑定查询/取消 |
| `mcp_audio_api.rs`、`audio_excerpt.py`、`python_bridge.rs`、`engine.py` / `bridge.py` | 注册文件按需解码；最长 15 秒 WAV/PCM16/16kHz/mono；两路音频读取；25 秒桥接期限；临时文件不入库；前后来源复核 |
| `core_ipc.rs`、`mcp_adapter/ipc.rs`、`mcp_adapter.rs` | 音频独立连接、35 秒期限；标准 MCP audio 块；请求取消/断连/停服回收；背压写入可中断；其余 IPC 保持 5 秒期限 |
| `mcp_write.rs`、`transcription.rs`、`mcp_operations_api.rs` | 所有者、引用及操作的收录范围；默认只读与桌面写授权；旧专题/选题仅保留内部兼容 |

桌面新增：

- `get_task_collection_states({taskIds}) -> [{task_id,asset_id,collected}]`，未登记任务省略。
- `set_task_collected({taskId,collected}) -> {task_id,asset_id,collected}`，沿原任务关联更新资产。
- `start_library_keyframes` 的 request 增加 `kind=range`、file_id、start_ms、end_ms、count。

MCP 新增：

- `start_frame_sampling`、`get_frame_sampling_status`、`cancel_frame_sampling`。
- `get_audio_excerpt`，返回请求/实际时间、文件和格式元数据，加标准 audio 内容块。
- 文案摘要增加实际 `file_id`，未知绑定为 null。默认工具共 20 项；`create_topic` / `create_idea` 退出公开路由，内部服务与历史数据保留。

完整参数、固定错误和授权规则见[桌面契约](workbench-knowledge-base-api.md)、[媒体契约](mcp-media-tools.md)、[读取规则](mcp-read-tools.md)与[写入规则](mcp-write-tools.md)。

## 基础门禁

| 命令 | 结果 |
| --- | --- |
| `npm run build` | PASSED；仍有现有大 bundle / use client 提示，无类型或构建错误 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib -- --test-threads=1` | PASSED，274 通过，2 个既有 GitHub 联网测试 ignored |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings` | PASSED |
| `.venv/Scripts/python.exe -m pytest -q tests/test_audio_excerpt.py tests/test_keyframes.py tests/test_transcription.py tests/test_transcript_timing.py tests/test_mcp_acceptance.py` | PASSED，62 通过 |
| `node --test tests/material-model-checks.test.mjs tests/mcp-client-checks.test.mjs` | PASSED，24 通过；拒绝失败文案、猜测文件绑定、重复/乱序/未交付帧的假通过 |

日志：`.local/materials-build-final.log`、`materials-rust-tests-final.log`、`materials-clippy-final.log`、`materials-python-tests.log`。

全量首轮发现四个旧测试仍默认下载后自动收录，已修订其前置为显式收藏或验证未收录恢复，未弱化资料保留断言；最后两轮全量均通过。一次重编测试时 Windows 链接器遇到仍在运行的旧测试 EXE，等待退出后重编通过，不计为代码验证通过。

## 详情收简复核

用户预览后要求：卡片采用可切换红心，详情右侧只保留可编辑的原始文案和 Markdown AI 总结；移除版本、复制导出、关键帧和笔记摘要入口；分类标签位于视频下方。复用原文的人工修改保存、笔记修订与来源链，不修改 schema、备份、MCP 视听接口，不删除历史资料。AI 总结用于写作、表达或分镜分析，本地不伪造 AI 生成结果。

本轮 UI 证据在 `.local/materials-ui-followup/`，使用隔离浏览器夹具。原文冲突恢复、写入后回读失败不重复提交、Markdown 安全预览、已有结构化 AI 分析保留、历史命中跳转、红心键盘切换与三个规定视口按当前界面重新检查。下面历史关键帧、片段、同步阅读及结构化编辑器浏览器记录属于此前实现状态，不表示当前默认详情仍提供这些入口；其持久数据与底层服务继续保留。

## 一级分类与收藏稳定性复核

分类改为平铺一级，列表内输入新建；隐藏已移出、导入视频与刷新图标。桌面创建/更新分类与 MCP 创建分类拒绝非空上级，读取 parent_id 为 null，分类搜索不再扩展历史后代；历史分类 ID、素材关联和备份结构保留，不迁移日常数据库。

心形收藏使用每个任务的独立等待状态，点击与焦点不传播到卡片选择，避免整页按钮禁用和卡片高亮闪动。浏览器夹具延迟收藏响应，检查其他卡片仍可操作、卡片选中态不变、心形不变淡；三个规定视口继续覆盖分类新建、筛选、标签批量保存、取消与错误恢复。证据位于 `.local/materials-flat-followup/`。

后续接入已按用户要求改为仅本机 Streamable HTTP，命令启动方式移除；本节为一级分类阶段记录，HTTP 当前证据见[独立记录](mcp-http-validation.md)。

当前增量验证：`npm run build` 通过；全量 Rust `275 passed / 2 ignored`。复核修正“旧子分类改名不得清除历史 parent”后，Core API 15 项通过，备份恢复仍保留旧关系，Clippy 全目标通过。官方 MCP SDK `--library` / `--writes` 通过；浏览器列表 7 组、边界 3 组、延迟收藏 1 组、10,001 任务 1 组、详情 6 组和精确搜索 1 组通过。Windows 原生使用独立 `materials-flat-20261029` 资料库重编译，原资料未覆盖；窗口启动与完整原生人工验收分别记录。macOS 和安装包不因此变为已验证。


## 浏览器与原生分别记录

使用既有 Playwright CLI / Edge、真实 React 和隔离 IPC 夹具。覆盖 1440×900、1280×800、1024×680，结果不替代 Tauri 原生窗口。

| 脚本 | 结果 |
| --- | --- |
| `knowledge-library.browser.js` / `knowledge-library-boundaries.browser.js` | PASSED，7 / 3 组；导航、筛选、分页、组织及错误反馈 |
| `library-asset-detail.browser.js` / `library-video.browser.js` | PASSED，6 / 7 组；文件状态、重关联、实际浏览器解码及释放 |
| `library-writing.browser.js` / `library-writing-failures.browser.js` | 本轮 PASSED，1 / 1 组；原文编辑、Markdown、历史内容、保存、冲突及未保存保护 |
| `library-backup.browser.js` | PASSED，三视口及恢复/取消/导出组合 |
| `library-keyframes.browser.js` / `library-clips.browser.js` | 此前基线 PASSED，7 / 9 组；当前默认详情已移除这些入口 |
| `material-collection.browser.js` / `material-many-tasks.browser.js` / `material-search.browser.js` | 本轮 PASSED；心形收藏/键盘切换/详情、10,001 任务拆批、精确历史文案与总结搜索 |
| `material-range.browser.js` | 此前基线 PASSED；范围采样入口现已隐藏，MCP 能力继续保留 |
| `task-feed.browser.js` / `task-platforms.browser.js` | PASSED，6 / 8 组；信息流卡片高度、封面动作、筛选、分页和键盘可达性 |
| `mcp-settings.browser.js` | PASSED；三视口、默认关闭、复制及错误/断连 |

Windows 隔离程序通过 `scripts/p1a-acceptance.ps1 -Scenario upgrade -RunId materials-20261029 -BuildOnly` 编译。首次启动并配置 WebView2 CDP 的组合命令被自动审批拒绝。用户随后明确要求编译启动，前端 build 与原生重编译通过，普通可见启动成功，进程可响应并具有“映取 P1A upgrade 隔离验收”窗口。窗口启动通过不等于完整原生交互、文件选择器或安装包验收通过。

macOS 服务可查询到的 Workspace catalog 仅登记其他项目，没有本仓库，不能在其权限外运行当前工作区差异。Apple Silicon / Intel 本轮均为 **UNAVAILABLE**；历史 Mac 记录不替代本次验证。安装包构建、安装后运行和新环境验收均 **NOT_RUN**。

## 官方 SDK、真实 Codex 与真实引擎

官方 TypeScript MCP SDK（真实 stdio/IPC/Core）通过：

- `node scripts/verify-mcp-adapter.mjs --media`：`.local/wb312/run-o4jIgI/summary.json`。
- `--operations`：`.local/wb311/run-v9uJkv/summary.json`。
- `--scope`：`.local/wb204/run-UPgfeR/summary.json`。
- `--resources`：`.local/wb205/run-1yJbA7/summary.json`。
- `--writes`：`.local/wb310/run-f0DB87/summary.json`。
- `--av`：真实 15 秒 WAV 被接收、解码为 240,000 帧 / 16kHz / mono / PCM16；0–15 秒五帧采样返回五张可用 PNG。输出小于 1 MiB。

真实 Codex 验证使用现有原生 `codex.exe` 的 app-server、当前用户授权账号、ephemeral 只读会话及隔离合成库；不复制凭据、不读取日常资料、不修改用户 MCP 配置，禁用继承的其他服务。版本 **codex-cli 0.162.1**，模型 **gpt-6.1-sol**。

最终证据：`.local/materials-av/run-SJqp6f/`、`.local/materials-codex/client-BYI2uR/`。模型实际检索并读取有内容的合成识别稿，分析问题式开头及观点/步骤递进；保留 asset/file/document/version。模型主动启动范围采样、等待完成、读取 PNG，正确描述红色画面及实际 PTS/帧引用，明确静帧不能证明连续运动。审计器修订后用保存的真实 trace 复验通过，没有为审计修订重复模型调用。

**音频分层结果：**SDK 解码通过；Codex 直接工具协议调用收到 `text + audio`；模型工具结果被客户端转成 `text + text`，模型明确 `audio_consumed=false`，未声称听到内容、音色、语速或语调。因此实际模型听音为 **UNAVAILABLE**，不能将服务交付或整轮文本/图片验证通过解释成模型听音通过。

独立 PyInstaller 引擎构建于 `.local/materials-engine/`；既有日常引擎未被覆盖。用该引擎、隔离复制的 Fun-ASR-Nano CPU 模型及仓库 smoke.wav 构造的合成视频运行 `timing_acceptance`，结果 `.local/materials-asr/report.json` 为 PASSED：真实转录、两个不可变版本、VAD 媒体时间、重开绑定和源文件不变。

## 数据、权限与资源结论

- schema v11 / backup v9 未改变；历史文案、笔记、比较、专题、选题、PNG 及引用往返测试通过。新增无 clip_id 的范围帧也通过 v9 恢复。
- 收藏/移出、同作品多文件、清理历史、文件缺失、重定位和重启的资料保留通过 Core/SQLite 回归。
- 已知资产、笔记、文档 ID 与 URI 不能访问未收录资料；历史比较的跨资产摘录及搜索命中也受同一校验。
- 音频成功、无音轨、超量/非法关系、源变化、真实超时、取消、断连与停服路径已验证。测试观察实际 FFmpeg/引擎退出与临时目录消失，音频解码期间其他连接能继续读取。
- 停服能中断客户端不读取大响应造成的背压，确定性 1 KiB duplex / 800 KiB 响应测试通过。
- M01–M06 独立只读子 Agent 分区审查已覆盖实现与验证脚本；发现的退出等待、历史笔记泄漏和验收假通过问题均已修复并复验。

## 明确剩余验收

1. 补 Windows 原生窗口完整“下载→收藏→整理→AI访问”交互。
2. 提供登记当前仓库及差异的 macOS 工作区，分别补 Apple Silicon / Intel 原生结果。
3. 在能消费标准 MCP audio 的客户端/模型验证实际听音；当前模型继续使用转录文字分析语义。
4. 如需发布，另行授权安装包、安装后验收、提交及发布；本轮未执行这些动作。
