# 工作台与知识库实施记录

## 2026-10-27：准备与首批核心实现

对应 [任务清单](workbench-knowledge-base-tasks.md) 的分批实施证据。当前已完成 WB-001～WB-003、WB-101～WB-118、WB-119 的 Windows 集成验收，以及 WB-201 的 SDK 选型与 Windows 协议 / 客户端验证、WB-202 的 Core 本机 IPC 与 Windows 命名管道验证、WB-203 的 stdio 适配与 Windows SDK / 进程回收验证，以及 WB-204 的七项只读工具与 Windows 分页 / 引用 / Unicode / 读取边界验证、WB-205 的受限 Resources 与 Windows 统一范围 / 隐私边界验证、WB-206 的 MCP 设置、原生配置复制与 Windows 中文空格路径客户端启动验证、WB-207 的安装包与 Windows 实际安装后启动 / 连接验证。WB-208 已完成 Windows 真实 Codex 客户端 / 模型组合、来源核对及生命周期验收。macOS Intel / Apple Silicon 仍待验收，WB-119 / WB-201～WB-208 总项保持未勾选；P1B 为 Windows 限定通过。下文各批记录保留当时的验证边界，WB-301 已完成 Windows 的真实时间来源与媒体原点验证，两种 macOS 继续待办。WB-302 已实现媒体 PTS 零点、段落时间存储和 Windows 验证，两种 macOS 继续待办。WB-303 已实现片段收藏与范围播放并完成 Windows 验证，macOS 两种架构继续待验收。WB-304 已完成关键帧与资源管理的 Windows 验证，两种 macOS 继续待验收。WB-305 已完成同步阅读与 Windows 验证，两种 macOS 继续待办。WB-306 已完成结构化拆解及 Windows 来源 / 引用验证，两种 macOS 继续待办。最新状态见[第二十四批](#第二十四批wb-306-结构化文案和制作手法拆解)。

### 工作区与改动边界

- 起始分支 `master`，HEAD `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`。
- 起始已有 `docs/README.md`、`docs/tauri-desktop-technical-plan.md` 的修改，以及未跟踪的技术方案和任务清单；均为此前本对话的文档成果，保留并继续维护。
- 新增 `src-tauri/src/library/`，调整核心初始化、成功下载发布、转录结果持久化和历史清理；前端只增加现有任务错误机制的知识资料保存失败文案。
- 不提交、推送或发布；不读取用户实际应用数据库，不使用真实 Cookie，也不在真实下载目录运行迁移。

### WB-001 调用路径核对

| 边界 | 当前入口及行为 | 本批处理 |
| --- | --- | --- |
| 数据库初始化与恢复 | `Core::open` 初始化任务、解析缓存、转录及设置，恢复提交证据和任务 | 在旧库修改前初始化知识库并执行一致性备份；恢复任务后事务回填资料 |
| 下载文件最终提交 | `start_download` 校验 / 提交文件后通过 `publish` 发布完成 | 完成状态持久化后事务入库；入库错误保持下载完成、显示错误，重启回填 |
| 历史清理与删除 | `remove_history_records` 清除任务、旧转录快照及最后任务引用的缓存封面；`delete_local_file` 成功删除文件后调用它 | 清理前事务保留资料；任务关系自动解除，独立文案及封面不随之删除 |
| 封面 | `parse_cache::cover` 或历史 `cover_path`，原清理按任务引用释放 | 将封面字节持有在 `asset_resources`，与解析缓存清理分开 |
| 转录 | `transcription::store` 保存以任务为入口的操作快照和成功结果 | 同事务保留独立识别文档；操作入口尚未切换，WB-110 保持未完成 |
| 桌面 / IPC | `desktop.rs` 注册 Tauri 命令，`src/api.ts` 调用，Rust `TaskEvent` 发布 | 新领域服务暂未注册前端 / MCP 入口，WB-112 及后续保持未完成 |

实际文件删除在当前源码中会移除下载任务，与旧技术方案首段表述的细节存在历史差异。本批保留现有任务行为，只新增独立知识资料保护；页面操作约定已在 `DESIGN.md` 第 12 节区分。

### 数据和兼容处理

- 使用现有 SQLite、UUID、SHA-256 和 Rust 标准库，不新增运行依赖。通过 SQLite `VACUUM INTO` 备份包含 WAL 中已提交内容的旧库，不直接复制主数据库文件。
- 备份位于应用数据库同目录，名称为 `<数据库主名>.pre-library-v1-<UUID>.sqlite3`，可能包含旧设置与凭据，仅供本机升级恢复，不能作为可分享资料包。
- 首批 schema 为版本 1；空库无需旧库备份，未知新版本拒绝打开。DDL 事务失败不留下半套表；历史回填独立事务，失败回滚并在下次启动重试。
- 原文与整理稿分别保存；整理稿每次保存建立不可变版本，版本冲突拒绝覆盖。来源引用保存稳定文档 ID，旧版仍可核对。
- 转录任务快照暂时保留旧成功正文，以维持现有弹窗与任务 API；独立资产已保留成功文案。待 WB-110 切换入口时移除长期正文双写，此过渡不能标记为 WB-110 完成。
- 已提交下载的入库错误只记录 SQLite 错误码、IO 类型及 JSON 分类，避免泄露路径或正文。即使附加警告也无法写入数据库，内存和数据库中的下载完成状态仍保留，重启从完成记录补齐资料。
- 不移动原媒体，不补造时间信息，不恢复已经被旧版本删除的文案。原生播放兼容、完整逻辑备份恢复及新增 UI 仍由后续任务交付。

### WB-003 隔离样本与自动验证

`library/tests.rs` 使用临时 SQLite / 临时媒体和合成任务，覆盖重复作品、多文件版本、旧模型身份、源文件缺失、图集、失败 / 预览、无封面及旧封面。`desktop_core.rs` 中新增真实 `Core` 生命周期测试及知识库保存失败注入；现有转录集成测试追加历史清理后的资产文案断言。

| 检查 | 状态 | 证据 / 范围 |
| --- | --- | --- |
| 知识库定向测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml library:: -- --test-threads=1`，8 项通过 |
| Rust 全部测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1`，143 通过、2 项既有在线检查忽略；末次窄修复另按下一行验证 |
| 最终失败恢复及核心集成测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml library_ -- --test-threads=1`，2 项通过，覆盖入库与额外警告同时写入失败的最终修复 |
| Rust 探针构建与 Python 集成 | PASSED | `cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe` 后 `.venv/Scripts/python.exe -m pytest -q`，448 项通过；最终警告分支窄修复由上述 Core 测试覆盖 |
| 前端类型与构建 | PASSED | `npm run build`；既有组件的 `use client` / sourcemap 及 bundle 体积警告仍存在 |
| Rust Clippy | PASSED | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`，最终窄修复后通过 |
| Rust 新模块格式 | PASSED | 对新增 `library/mod.rs` 及其子模块运行 rustfmt，未格式化全仓库 |
| 文档、任务依赖和差异检查 | PASSED | `git diff --check`；脚本检查新文件空白、相对链接、49 个任务编号和无环依赖；完成项均满足前置任务 |
| 独立代码审查 | PASSED | CHILD_AGENT / Tier 3，11 个代码文件、跨模块集成及最终文档完整覆盖；无剩余 P0/P1/P2，诊断与警告写入失败的修复已复审 |

最终代码目标 SHA-256：`9da0107fc8e6cf5e296a6ac44ece690ca88b4ab1e9c175f326b968d5b4a73692`。目标按 11 个代码文件的路径与内容哈希计算，包含未跟踪的新模块；审查后仅回填本行审查结论，代码未改变。

全量 Rust 首次运行有 1 项既有 Telegram 测试在读取追加中的测试事件 JSONL 时报 EOF；独立重跑及后续全量均通过，没有修改该测试或将失败忽略。新模块定向测试、全量测试、最终窄修复定向测试分别保留上述证据，不将某次失败隐藏为一次全量成功。

### 平台和人工验收

| 场景 | 状态 | 说明 |
| --- | --- | --- |
| Windows 隔离核心测试 | PASSED | 当前本机编译运行，Rust / Python 均使用隔离 fixture，不触及真实资料 |
| 浏览器知识库页面 | NOT_RUN | 页面尚未实现 |
| Windows 原生 Tauri 新流程 | NOT_RUN | 本批核心验证不替代原生页面验收 |
| macOS Intel / Apple Silicon | UNAVAILABLE | 当前为 Windows 环境，分别待对应平台验证 |
| 安装包升级与回退 | NOT_RUN | 尚未生成本批安装包 |
| 真实 MCP 客户端 | NOT_RUN | MCP 属于后续 P1B |
| 人工视觉 / 视听和总结质量 | NOT_RUN | 需要对应产品流程可用后单独验收 |

### 旧库恢复边界

需要回退到旧应用时，先退出应用和所有数据库访问进程，保留升级后数据库及其 WAL / SHM，再按备份文件恢复到独立验证目录检查完整性，最后由明确的恢复操作替换旧库。不可在应用运行时直接覆盖 SQLite 主文件，也不可把升级前备份当成包含升级后新笔记的副本。本记录不执行用户实际数据库回退。

## 2026-10-27：本地导入与资产转录

对应 WB-109、WB-110。沿用第一批未提交工作区，本批开始前的源码快照位于忽略目录 `.local/workbench-batch2-baseline/`，用于区分本批增量与此前已审查成果。未提交、推送或发布，未读取真实应用数据库。

### 实现和数据边界

- `library_import.rs` 复用 Core 的 worker、取消、退出和暂存回收，执行独占导入 / 重新关联操作。导入引用原文件，识别已有路径或物理文件（包括硬链接），不创建虚构下载任务。
- 原生命令 `import_library_video` / `relink_library_file` 打开文件选择器，取消选择直接返回 `null`；选定文件由 Rust 进行路径和指纹确认，再通过 sidecar 调用 FFprobe。新知识库页面入口尚未实现，不把原生命令可用宣称为完整页面交付。
- `local_video.py` 只开放 file 协议及 mov / matroska / webm 容器，拒绝播放列表、纯音频、附带封面冒充视频、无有效尺寸及损坏内容；未知时长保持未知。FFprobe 20 秒限时，整个 sidecar 操作 40 秒限时，取消 / 退出回收实际进程后清理暂存。
- 重新关联同样先探测，失败保持原关联；成功保持资产和文件 ID，更新文件元数据，保留文案原始来源指纹。原文件 A 仍存在而关联 B 时，旧文案必须标记来源变化，不能只检查 A 是否变化。
- 新 `asset_transcriptions` 表只存状态摘要。识别结果和其源文件、模型信息唯一存于 `asset_documents`，运行中或完成状态的持久化 JSON 中 `result=null`。旧任务命令和 `transcription_updated` 事件保持兼容，新增资产命令和 `asset_transcription_updated` 事件供后续页面使用。
- 本地视频不依赖下载历史即可提取文案。单转录并发、CPU / CUDA 模型身份、取消、退出回收、失败 / 无语音保留成功原文、旧版模型信息和整理稿保留均继续有效。列表摘要读取不需要加载成功全文。

### schema 和 sidecar 升级

- schema 从 1 升至 2，升级前通过 `VACUUM INTO` 生成 `<数据库主名>.pre-library-v2-<UUID>.sqlite3`；未知未来版本拒绝打开。
- 启动事务回填时迁移旧任务正文和操作状态，保留文件、文档和模型信息，再删除对应旧 `video_transcripts` 行；重复启动不重建正文。该旧表仅用于接收待迁移旧记录，不作为新操作写入目的地。
- 新增 `probe_local_video` / `local_video_probed` 操作，JSONL 协议保持 v3；Rust、Python 和离线 fixture 的引擎版本同步为 0.15.0。模型、推理算法和下载策略未变。发布前必须重建和验证打包 sidecar，本批没有生成安装包。
- 第一节记录的 schema 1、旧全文双写过渡及其目标哈希属于第一批历史证据；当前行为以本节为准。

### 验证矩阵

| 检查 | 状态 | 证据 / 范围 |
| --- | --- | --- |
| 知识库和导入定向 Rust 测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml library -- --test-threads=1`，14 项通过，覆盖导入、物理文件去重、变更中止、取消 / 退出、v1 升级与原有资料保留 |
| 旧转录入口回归 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml transcription -- --test-threads=1`，8 项通过；对应测试已按新存储表注入失败并保留原断言 |
| Python 探测、封面与转录定向测试 | PASSED | `.venv/Scripts/python.exe -m pytest -q tests/test_local_video.py tests/test_local_cover.py tests/test_transcription.py`，50 项通过；包括真实 FFmpeg 生成短视频后 FFprobe 验证和损坏容器拒绝 |
| 关联修复后的定向测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml library_import -- --test-threads=1`，3 项通过，覆盖 A 保留、关联 B 的来源标记及无效 B 保持原关联 |
| Rust 全部测试 / 探针构建 / Python 全部测试 | PASSED | 全量 Rust 147 通过、2 项原有在线检查忽略；构建 m1-probe 后 pytest 463 项通过。关联修复由上述最后定向检查覆盖 |
| 前端类型与构建 | PASSED | `npm run build`，关联接口新增后重跑通过；原有 `use client` / sourcemap / bundle 体积警告保留 |
| 桌面特性 Clippy | PASSED | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`，关联修复后通过 |
| 文档 / 任务 / 格式检查 | PASSED | `git diff --check`、新增 / 重构 Rust 模块 rustfmt、脚本校验新文件空白、相对链接和 49 项任务依赖；13 项完成，前置任务均满足 |
| 独立审查 | PASSED | CHILD_AGENT / Tier 3，完整覆盖本批 19 个代码文件、集成及最终文档；关联后来源标记 P1 已修复并新增有效回归，无剩余 P0/P1/P2 |

第二批最终代码目标 SHA-256：`6465046bf24881801151f41a9f6f2fd3ca25b949e2acbf3973d75e59820d5dbb`，按相对本批基线变更的 19 个代码文件计算并独立复算一致。审查后仅回填此结论，代码未改变。

Windows 原生文件选择器人工操作、知识库页面、安装包、真实 ASR 模型推理和最终人工视听均为 `NOT_RUN`；macOS Intel / Apple Silicon 为 `UNAVAILABLE`，并非自动测试覆盖。原生选择器代码复用现有对话框回调与取消路径，完整页面和原生验收仍由 WB-113～WB-119 负责。

## 2026-10-27：中文检索与完整桌面命令

对应 WB-111、WB-112。本批开始前工作区与第二批交付一致，HEAD 仍为 `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`；增量源码基线位于忽略目录 `.local/workbench-batch3-baseline/`。没有提交、推送或发布，没有读取真实应用资料库。schema 仍为 2，引擎仍为 0.15.0，无新依赖或 DDL。

### 实现与契约

- `search.rs` / `search.sql` 实现中文及字面子串查询、分类 OR 含后代、标签 AND、归档、平台、文件状态和稳定键集分页。全文查询包括保留的旧版本，命中携带稳定文档 ID / 版本；每资产一个至多 120 字符的摘要，不返回整篇正文。
- `views.rs` 提供资产详情和文案 / 笔记摘要，指定 ID 才读取全文；校验资产归属并保留模型与原始文件信息，文件状态和文案来源差异以当前关联为准。
- `library_api.rs` 将读写集中到 Core，同步已有字段和事务机制；SQL / 存储 JSON 错误规范为 `DATABASE_ERROR` 并保留诊断链，业务冲突保留具体错误码。人工笔记输入不能伪造 AI 来源。
- `library_commands.rs` 注册 23 条新增 Tauri 命令，`src/lib/library-api.ts` 提供相应强类型调用；`src/api.ts` 导出 `libraryApi` 并补齐中文错误反馈。导入、重新关联和资产转录继续使用第二批入口。检索使用框架已有异步命令调度，不引入额外线程池。
- `library_changed` 事件只发送范围及资产 ID；成功写入、新下载入库、导入 / 重新关联、成功文案和旧下载入口清理 / 删除均接通。分类或标签全局变化要求刷新相关组织与资产显示；不发送全文或进度正文，不提供任意 SQL 或任意路径读取。
- 详细命令、DTO、筛选、文件状态、摘要和事件语义见 [API 契约](workbench-knowledge-base-api.md)。本批不包含新页面、永久删除资料、WB-117 导出备份或 MCP 实现。

### 检索负载记录

Windows 本机 debug 测试，在内存 SQLite 中构造 1000 条资产、1000 份整理稿，SQL 实测文本总量为 16,218,000 bytes；每资产登记一个隔离的缺失文件路径。查询词“灵感检索样本”命中全文，读取默认首页 20 条及下一页标记，包含实际命中摘要和该页文件状态计算。三次查询分别为 106 / 106 / 103 ms，样本构造为 65 ms。

这是明确构造的负载，不是用户真实资料库、端到端 UI 或跨平台性能证明。本样本下暂不需要 FTS / 向量索引；后续依据实际体量和使用体验继续判断，不外推到十万条或长文案极限。

### 验证与审查

| 检查 | 状态 | 证据 / 范围 |
| --- | --- | --- |
| 知识库定向测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml library -- --test-threads=1 --nocapture`，22 项通过，包含检索、Core 命令、既有导入迁移与体量输出 |
| Rust 全部测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1`，155 通过、2 项既有在线检查忽略；之后的删除通知修复见下一行 |
| 最终修复定向测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml library_api -- --test-threads=1`，4 项通过；新增旧下载入口删除视频后的知识库文件状态 / 事件断言 |
| Rust 探针与 Python 集成 | PASSED | 构建 m1-probe 后 `.venv/Scripts/python.exe -m pytest -q`，463 项通过；未修改 Python 协议或算法 |
| 桌面特性 Clippy | PASSED | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`，包括全部命令注册及最终修复 |
| 类型与前端构建 | PASSED | `npm run build`；既有组件 `use client` / sourcemap 和 bundle 体积警告保留 |
| 格式、文档及任务检查 | PASSED | `git diff --check`、本批新增 / 重构 Rust 模块 rustfmt；脚本核对新增文件空白、相对链接、49 个唯一任务编号、无环依赖及完成项前置条件，15 项完成 |
| 独立审查 | PASSED | CHILD_AGENT / Tier 3，15 个代码文件、跨模块集成及最终文档完整覆盖；旧下载删除入口缺少资产失效通知的 P1 已修复并复审，无剩余 P0/P1/P2 |

审查发现的删除通知缺口已在 `remove_history_records` 事务中收集相关资产 ID，提交成功后去重排序通知；保留原有任务删除事件。新增测试从真实 Core 下载记录删除入口执行文件删除，验证资产仍在、文件显示缺失且收到正确 Assets 摘要。测试样本补齐生产 `canonical_url` 字段后通过，未弱化原断言。

第三批最终代码目标 SHA-256：`d5cbb5bf6ed6362d495bac241886c786b5d60697126fac45295953280d0b0f72`，按本批 15 个代码文件的路径与内容哈希计算，包含未跟踪的新模块，独立复算一致。最终文档已审查；审查后仅回填本段和审查结论，代码未改变。

本批 Core 行为、DTO 序列化 / 反序列化、Tauri 宏注册编译及 TypeScript 调用均已验证。未运行 GUI 中的真实 IPC 点击、原生播放器 / 定位、知识库新页面、安装包和 macOS；这些保持 `NOT_RUN` / `UNAVAILABLE`，不以构建或 mock 替代人工与平台验收。

## 2026-10-27：导航与知识库列表

对应 WB-113，完成后共 16 / 49 项。沿用前三批未提交工作区，起始分支 `master`、HEAD `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`；本批基线与文件哈希位于忽略目录 `.local/workbench-batch4-baseline/`。不提交、推送或发布，不读取实际应用数据库、视频或凭据。没有新增依赖、修改 Rust / Python、schema 或 IPC 契约。

### 实现与范围

- `main.tsx` 新增工作台、知识库导航，将原下载作品入口命名为下载中心，下载任务缩进展示；下载输入、解析预览、后台任务和全局事件状态保留。工作台目前提供知识库与下载页面的真实跳转，WB-116 的最近访问、待整理和访问持久化尚未实现。
- `knowledge-library.tsx` 使用真实 `libraryApi` 实现搜索、分类 OR / 标签 AND、平台 / 文件筛选、待分类 / 已移出、分页、单选 / 全选已加载项、导入及取消、批量归档 / 恢复。列表保持摘要，不加载全部文案；未知作者和封面缺失如实展示。资产详情、视频播放、重新关联和正文编辑仍由 WB-114 / WB-115 接入。
- `library-organization.tsx` 使用现有 Dialog、Checkbox、Select、Input 管理分类和标签。分类支持新建、重命名、移动和排序，自身 / 后代不能作为上级，删除有子分类时保留核心错误；名称最多 128 个 Unicode 字符。单个或多个视频可以勾选多个分类标签，混合关联显示部分已选。
- 每项关系沿用一个核心原子批量命令，不把多项命令宣称为同一事务；部分成功显示已保存项数，保留剩余修改，重试不会重复已成功命令。错误持续显示在页面或弹窗，保存期间阻止重复操作。
- `use-library.ts` 在进入页面时订阅变更并并行读取摘要与组织列表；离开时解除订阅，返回 / 聚焦 / 事件刷新。请求代次阻止旧响应覆盖新查询，分页防重。组织读取不被检索失败阻断，删除当前已选分类 / 标签后自动清除失效筛选；订阅失败持续显示。
- CSS 复用 `DESIGN.md` 的 Radix 语义 token、12 / 14 / 20px 字号与 0.5px 边界，不新增主题或动效。宽视口并列分类栏，1024 × 680 用按钮折叠；筛选固定，列表独立滚动，组织弹窗保留可达底部动作及关闭焦点。
- 10 份既有浏览器脚本仅同步导航 / 标题“下载作品”到“下载中心”，保留原断言；不因本批功能改变其他历史交互。

### 验证与审查

| 检查 | 状态 | 证据 / 范围 |
| --- | --- | --- |
| 类型与生产构建 | PASSED | `npm run build`；最终生产实现修改后通过，原有 `use client` / sourcemap 和 bundle 大小警告保留 |
| 知识库浏览器流程 | PASSED | `playwright-cli -s=wb113 --raw run-code --filename=tests/knowledge-library.browser.js`，Edge 隔离会话，7 组通过，含 1440 × 900、1280 × 800、1024 × 680、分页、组合筛选、空态、组织 CRUD、删除已选筛选、批量失败重试、归档恢复、事件订阅清理、旧响应丢弃及导入取消 |
| 知识库边界与键盘 | PASSED | `tests/knowledge-library-boundaries.browser.js`，3 组通过；128 字符、无效上级排除、窄窗弹窗底部 / Esc 焦点、部分成功后只重试未保存关系、导航切换保留独立输入与筛选 |
| 原后台下载回归 | PASSED | 从 `tests/fixtures/background-download.html` 运行 `tests/background-download.browser.js`，15 组通过，覆盖原平台、队列、重复确认、失败、迟到事件和三视口键盘路径 |
| 截图与控制台 | PASSED | 三视口列表及 1024 分类弹窗已目视，无横向溢出；本批隔离 fixture 控制台没有新增错误。截图位于忽略目录 `.local/wb113-*.png` |
| 格式、文档和任务依赖 | PASSED | `git diff --check`，脚本检查新增代码 / fixture 空白、相对链接、49 个唯一编号、无环依赖及 16 个完成项的前置条件 |
| 独立审查 | PASSED | CHILD_AGENT，完整覆盖 18 个代码 / 测试文件、集成及 6 份最终文档增量；检查点问题和最后取消语义均已复审，无剩余 P0/P1/P2 |

检查点发现“检索失败丢弃组织读取，删除已选分类或标签后不能恢复”的 P1，已通过 `allSettled` 和剔除失效筛选修复；主浏览器脚本覆盖该路径。名称上限与核心不一致的 P2 已修复，边界脚本验证 129 字符拒绝且 128 字符可保存。首次主脚本因空态和工具栏重复“清除筛选”导致严格定位失败，界面移除冗余入口后通过。后台下载初次从错误 fixture 启动导致等待超时，按脚本指定入口运行后 15 组通过。

最终核对原生取消契约时，将 fixture 的运行中导入取消改为与 Core 一致的 `CANCELED` 拒绝；页面对此显示“已取消导入”，不套用依赖管理的取消错误文案。最后重新运行构建和主浏览器 7 组通过，包含取消不显示失败 Alert 的断言。

第四批最终代码目标 SHA-256：`d853a038eec113e04d08e372d87ebbf9b1de217c7502ee0a161f9253074cbce4`，按本批 18 个代码 / 测试文件的路径和内容哈希计算，包含新增未跟踪文件，独立复算一致。最终文档已核验，审查后仅回填本段及审查结论，代码未改变。

额外尝试既有 `tests/task-platforms.browser.js`，末段要求筛选后立即挂载 51 行，当前既有 `TaskList` 首页只挂载 30 行，故报告 `fixed tab cannot filter long list`。`task-list.tsx` 与本批基线相同，该断言与既有懒加载契约不一致；本批不修改此实现或弱化断言，不将该脚本计为通过。其余未运行的旧浏览器脚本不宣称完成回归。

### 真实运行边界

浏览器使用 `tests/fixtures/knowledge-library.html` 的合成资料和 IPC mock，存储仅使用专用测试键。页面重载验证 fixture 保存 / 重新读取，不代表真实 Tauri 重启；核心的持久化契约沿用前三批隔离 Rust 验证。本批不再运行未受影响的 Rust / Python 全量测试。

Windows 原生文件选择器、真实 Tauri GUI / 重启 / 文件系统与安装包为 `NOT_RUN`；macOS Intel、Apple Silicon 为 `UNAVAILABLE`。本批没有播放器或正文界面，未宣称完成 P1A 或人工视听验收。

## 2026-10-27：资产详情与视频播放

对应 WB-114，完成后共 17 / 49 项。起始分支 `master`、HEAD `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，沿用前四批未提交成果；本批增量基线位于忽略目录 `.local/workbench-batch5-baseline/`。没有提交、推送、发布、新增依赖或修改 schema，也未读取用户实际资料库、视频或凭据。

### 实现与边界

- 列表标题成为可访问的详情入口。`library-asset-detail.tsx` 展示来源 / 作者 / 收录时间、分类标签、文案版本与笔记摘要；可继续修改分类标签，切换真实文件版本并查看路径、规格、缺失 / 变化状态。后退保留列表筛选并恢复触发器或列表焦点。WB-115 的正文阅读和编辑不在本批实现。
- 播放使用新 `library-video.tsx` 组合原生 `<video>` 和现有播放器样式，复用单文件准备命令与 asset URL；媒体不通过 IPC 整体传输。按文件登记指纹隔离播放会话，切换版本、返回、离开知识库、文件确认缺失时暂停并清除 src，迟到响应不重开。库内播放本批不持久化播放位置；下载任务播放器及其进度契约保持原实现。
- 现有 `LibraryCover` 提取为共享组件，保持封面 URL 在卸载 / 更新时释放。详情采用 2:1 媒体 / 资料布局，窄窗口纵向组织，独立滚动；复用 Radix 颜色、三种字号和 0.5px 边界，未新增装饰动画。
- 增加第 24 条知识库原生命令 `open_library_file` / `libraryApi.open`，复用 `Core.library_playback_path` 校验资产和文件归属、普通文件与缺失，再调用现有 opener。仅接受 ID，不接受任意路径。内置播放失败保留可见错误，由用户选择系统播放器；打开前暂停内置媒体。真实系统启动失败保留错误。
- 重新关联复用原生选择器、探测及取消机制，成功刷新路径、规格与状态并保持文件 ID 和资料；失败 / 取消保留原关联。缺失视频仍可查看来源、分类和资料摘要；未知规格如实显示。状态在查询、窗口聚焦或已有事件后刷新，不新增文件系统监视器。

### 验证矩阵

| 检查 | 状态 | 证据 / 范围 |
| --- | --- | --- |
| Core API 定向测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml library_api -- --test-threads=1`，5 项通过；新增超过 260 字符的中文路径、错误资产 / 文件归属、删除后 `FILE_MISSING` 及资料仍在的真实 Windows 临时文件断言 |
| 桌面命令与 Clippy | PASSED | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`，包含新命令及注册编译 |
| 类型与生产构建 | PASSED | `npm run build`，最终生产修改后通过；既有 `use client` / sourcemap / bundle 大小警告仍存在 |
| 详情浏览器交互 | PASSED | `tests/library-asset-detail.browser.js`，Edge 隔离会话 6 组通过；三个规定视口、标题 Enter / 内部 Ctrl+End 滚动及返回焦点、中文长路径、UTC 秒日期、文件版本、来源复制 / 定位、分类标签、未知 / 缺失 / 变化、重新关联成功 / 失败 / 取消及错误重试 |
| 实际浏览器解码及资源释放 | PASSED | `tests/library-video.browser.js`，7 组通过；三视口 VP8/WebM 160 × 90 和 H.264/MP4 90 × 160 解码、原生控制栏、拖动 / 暂停 / 音量、Range 请求、等比展示、切换 / 后退 / 导航释放、音频无画面 / 损坏 / 来源失败的手动回退、缺失刷新和迟到响应丢弃 |
| 原知识库回归 | PASSED | `tests/knowledge-library.browser.js` 7 组、`tests/knowledge-library-boundaries.browser.js` 3 组通过，原 WB-113 的列表、分页、筛选、分类标签及失败边界仍有效 |
| 截图与媒体来源 | PASSED | 已目视三视口详情及竖屏播放、窄窗底部，截图位于 `.local/wb114-*.png`。新增 MP4 为 FFmpeg 合成测试图，FFprobe 确认 H.264、90 × 160、4.000 秒、32932 bytes；生成方法见 `tests/fixtures/media/README.md` |
| 格式 / 文档 / 任务依赖 | PASSED | `git diff --check`、本批命令与测试模块 rustfmt；脚本核对新增文本空白、相对链接、49 个唯一任务编号、无环依赖及 17 个完成项前置条件 |
| 独立审查 | PASSED | CHILD_AGENT，完整覆盖 15 个代码 / 测试 / 媒体文件、集成及 7 份文档增量；时间戳 P1 已修复并复审，无剩余 P0/P1/P2 |

原生命令调用、文件选择器、定位和系统播放器在浏览器中使用隔离 mock；真实解码通过 Vite 的测试媒体 URL 完成，不证明 Tauri asset 协议或系统播放器已在 GUI 运行。资源释放断言检查已卸载的真实 HTMLVideoElement 处于暂停且无 src，非仅检查节点消失。

检查点审查发现收录时间把 UTC 秒当成毫秒而显示 1970 年，已统一乘 1000，并用真实 2026 秒级样本和本地显示断言防止回归。首次详情脚本的长路径样本不足 260 字符，已扩大合成路径并保留原长度断言后通过。最初手动 CLI 点击早于页面挂载而未定位到控件，正式浏览器脚本按可用控件等待，最终 6 + 7 组均通过。

第五批最终代码目标 SHA-256：`3092b4d869a2665f90c8da657e031186aa804d54901fa41ad48e9ae0b07ea890`，按本批 15 个代码 / 测试 / 媒体文件的路径和内容哈希计算，包含新增未跟踪文件及合成 MP4，独立复算一致。最终文档已审查；审查后仅回填本段与结论，代码未改变。

### 平台与人工验收

Windows Core 临时文件 / 路径检查和 Edge 实际媒体解码分别按上表记录。Windows 原生 Tauri GUI、asset 协议读取、真实选择器、系统播放器 / 文件定位、安装包及用户资料重启为 `NOT_RUN`；macOS Intel / Apple Silicon 为 `UNAVAILABLE`。本批不修改下载算法、Python 或持久化协议，不重复未受影响的 Python / Rust 全量测试，也不宣称完成 P1A 阶段或人工视听验收。

## 2026-10-27：文案编辑与笔记

对应 WB-115，完成后共 18 / 49 项。本批开始于 `master`，HEAD 仍为 `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`；增量基线位于忽略目录 `.local/workbench-batch6-baseline/`，保留前五批未提交成果。未提交、推送或发布，没有使用真实用户数据库、视频、剪贴板或凭据，无新依赖、schema 或 Python 协议变更。

### 实现与范围

- `library-writing.tsx` / `use-library-writing.ts` 在详情中提供识别原文、整理稿、笔记 Tab 和可访问的正文面板。全文只按所选资产 / 内容 ID 读取；支持长文滚动、完整复制、历史版本及原文模型 / 后端 / 来源核对。整理稿展示其来源原文信息，未知模型保留未知，当前文件与原始来源指纹不同会提示旧结果状态。
- 整理稿显式保存为不可变新版本，可从指定原文创建。普通刷新、重新提取或更新列表不覆盖当前输入；保存失败保留输入，冲突明确拒绝。显式读取最新内容时，把原输入放入可复制的恢复区，恢复副本也纳入未保存导航保护。
- 笔记支持新建、人工来源和类型选择，已有笔记按 revision 修改正文并保留创建来源、类型及引用。选中已保存版本的原句可创建引用笔记，引用保留文档 ID 和准确原句，按需打开只读原文；新建时 Core 继续校验归属 / 引用，不能伪造 AI 来源。
- `library-editor-navigation.tsx` 负责应用内离开保护：返回、页面导航、Tab / 版本 / 笔记切换仅在有未保存内容时确认，提供保存后离开、放弃修改、继续编辑。保存中不放弃提交，导航等待结果；失败停留原页。浏览器重载使用 beforeunload，未保存草稿不自动持久化，真实 Tauri 退出行为本批未验收。
- 写入成功后的回读失败单独报告“内容已保存”，保留正文和已知修订号，提供直接重读；重读不重复保存，也不会因沿用旧修订号制造额外冲突。首次内容读取失败同样可直接重试。保存冲突后的重新读取再次失败时，原输入仍在。
- 原文页复用资产转录入口、单并发、取消和摘要事件，显示真实分段进度。重新提取只增加识别版本，已有人工稿、笔记和未保存输入保持原状；切换文件或离开时解除 UI 订阅，后台任务沿用原生命周期。真实模型下载和推理仍通过已有设置与 Core 处理。
- 新增第 25 条命令 `export_library_document` / `libraryApi.exportDocument`，按资产 / 文案 ID 导出所选已保存版本。原生另存为取消返回 `null`；复用原 TXT 导出的 UTF-8 BOM、平台换行、临时文件提交和不覆盖来源校验，保护该资产全部当前文件及历史原文来源路径，包括硬链接别名。复制可用于当前草稿，未保存时禁用 TXT；Markdown / JSON 和逻辑备份由 WB-117 实施。
- 详情的媒体与正文改为等宽两列，1024 × 680 纵向排列；正文内部滚动，保存动作在正文下方，长引用限制在自身滚动区域。继续使用既有 Radix token、三种文字尺寸、0.5px 边界和现有控件，无新动画或通用编辑器依赖。

### 验证矩阵

| 检查 | 状态 | 证据 / 范围 |
| --- | --- | --- |
| Core API 测试 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml library_api -- --test-threads=1`，6 项通过；新断言验证所选版本的中文 / emoji / 换行与 BOM、归属错误不创建文件、原始及重新关联媒体和硬链接别名不被覆盖 |
| 原转录 / 导出回归 | PASSED | `cargo test --locked --manifest-path src-tauri/Cargo.toml transcription -- --test-threads=1`，8 项通过；覆盖原导出、缺失视频保护、历史 / 重启、模型身份、取消和失败保留 |
| 桌面 Clippy | PASSED | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`，新增命令与注册编译通过 |
| 类型与生产构建 | PASSED | `npm run build`，最终修改后通过；既有 `use client` / sourcemap 与 bundle 大小警告保留 |
| 编辑主流程 | PASSED | `tests/library-writing.browser.js`，最终 5 组通过；三个设计视口、长正文只读与滚动 / 全文复制 / TXT、旧模型及版本、整理稿来源与新版本、导航三选、Unicode 引用、笔记创建和修订 |
| 编辑失败边界 | PASSED | `tests/library-writing-failures.browser.js`，9 组通过；保存失败及离开失败、文案 / 笔记冲突、读取最新时失败与恢复副本、提交后回读失败 / 初读失败直接重试、修订号推进、ASR 进度 / 新版不覆盖人工稿、来源缺失 / 等待读取中重新关联、TXT 取消 / 失败及提交中的导航 |
| 原作者作品回归 | PASSED | `tests/author-profile.browser.js`，6 平台样本与 1440 × 900 / 1280 × 800 / 1024 × 680 通过，pageerror 为空；作者条件不被新导航清除 |
| 原详情和播放回归 | PASSED | `tests/library-asset-detail.browser.js` 6 组、`tests/library-video.browser.js` 7 组通过，原资料、文件版本、关联、真实 VP8 / H.264 浏览器解码及资源释放有效 |
| 截图与文案检查 | PASSED | 已目视三视口编辑及 1024 未保存弹窗，截图 `.local/wb115-editor-*.png`、`.local/wb115-unsaved-1024.png`；修正放弃按钮对比度，操作可读且三选完整 |
| 格式 / 文档 / 任务依赖 | PASSED | `git diff --check`、本批 API / 命令 / 测试模块 rustfmt；脚本核对新文本空白、相对链接、49 个唯一编号、无环依赖与 18 个完成项前置条件，不格式化无关模块 |
| 独立审查 | PASSED | CHILD_AGENT，完整覆盖 17 个代码 / fixture / 测试文件、调用上下文及 6 份文档增量；检查点问题和最终放弃按钮修复已复审，无剩余 P0/P1/P2 |

### 修复与验证记录

检查点发现作者作品跳转会清空已设置筛选的 P1，已将搜索重置限定于侧栏导航，专用作者跳转保留条件，原作者回归通过。提交成功但回读失败的 P2 已补充已知 revision 推进和直接重读入口；首次笔记读取失败也可直接重试，成功回读清除旧重试状态。等待读取期间重新关联文件的来源提示缺口，已通过当前路径 / 大小 / mtime 与原始指纹直接比较修复，没有新增并发或恢复抽象；相应边界均有断言。

首次引用测试的键盘选择未形成选区，改用控件的准确 UTF-16 选区和 select 事件后保留 emoji 原句校验，通过。初版脚本尝试在未保存时直接重载，并自动接受 beforeunload，与 CLI 模态处理冲突；测试改为通过界面的放弃操作清理 fixture 后重新加载，保护逻辑未移除。一次类型检查发现冲突回读 catch 引用了作用域外变量，修正后增加“冲突回读再次失败仍保留输入”断言并通过最终构建。视觉检查发现 AlertDialogAction 的默认文字色与 outline 变体冲突，放弃动作改用现有 Button 并保持受控关闭，最终主流程 5 组和弹窗目视通过。

第六批最终代码目标 SHA-256：`419cb9cd913aa4d695d68a446cc6c9998925e97a889f3e3a1f496352bf1adbc6`，按本批 17 个代码 / fixture / 测试文件的路径及内容哈希计算，包含新增未跟踪文件，独立复算一致。最终文档已审查；审查后仅回填本段和审查结论，代码未改变。

### 平台与人工验收

浏览器测试使用 `?detail=1&writing=1` 合成文案和 revision / 事件 mock；“重新提取”是状态与 UI 保留测试，不是真实 ASR 推理。复制和原生导出选择器 / 路径结果由 fixture 代替；真实文件编码、归属及覆盖保护由 Windows 临时目录 Core 测试验证。没有修改用户资料，也未将浏览器重载当成真实应用重启。

真实 Tauri 编辑、系统剪贴板、导出选择器、退出与重启、真实 ASR 模型及安装包为 `NOT_RUN`；macOS Intel / Apple Silicon 为 `UNAVAILABLE`。本批未重复未受影响的 Python 和 Rust 全量测试，不宣称 P1A 阶段或人工总结 / 视听质量已完成。


## 2026-10-27：基础工作台

对应 WB-116。本批保留起始 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0` 及前六批所有未提交成果，没有提交、推送或接触真实用户数据库。任务清单现为 19 / 49；WB-117～WB-119、备份恢复与 MCP 保持未完成。

### 实现与状态契约

- `library/schema-v3.sql`、`library/workbench.rs` 持久化每资产访问时间与单调访问序号；同秒访问仍可排序，重复打开只更新一条，v1 / v2 升级有 WAL 一致性备份和事务回滚。读取元数据不等于访问；详情成功打开后单独登记，后台刷新不更新顺序，保存失败仍可阅读并单独重试。
- 工作台三个资产分区各取六项与真实总数，排除归档资产。待分类为没有分类关系，待整理为没有 edited 版本且没有任何笔记；识别原文、文件缺失及更新时间不代替人工整理。完整待办入口复用同一搜索条件。
- `workbench.rs` 从 Core 现有下载任务、转录摘要 / 活动操作及本地导入生成操作 DTO；不另建任务队列，不返回正文、媒体 URL 或本地路径。导入最近一次失败只在本次进程内保留，下一次开始时清除；取消不计为失败。
- `workbench.tsx` 展示最近访问、待分类、待整理、运行中和需要处理项。转录定位至资产与文件版本；下载重读后按任务 ID 定位，清除原筛选并使分页外任务可达；删除后的任务留在工作台并反馈。导入返回原入口，可继续取消。
- 复用既有导航保护、按钮及 Radix token。两个宽视口为两列，1024 × 680 为单列；条目支持键盘打开，长标题省略，主区域滚动。事件、转录阶段 / 终态、任务状态或窗口聚焦刷新；失败保留上次结果，空库不展示合成资料。

### 实际验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml library -- --test-threads=1` | PASSED，29 项；新增 4 项工作台 Core 测试和导入失败断言，覆盖访问去重 / 同秒排序 / SQLite 关闭重开、失败写入无成功事件、v2 WAL 备份与回滚、pending 派生 / 检索、ASR 活动与未落盘状态覆盖、下载任务真实 ID |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASSED，包含 Tauri 命令与注册 |
| `npm run build` | PASSED，TypeScript 和 Vite；沿用已有 use-client / sourcemap 与包体积提示 |
| `tests/workbench.browser.js` | PASSED，1440 × 900、1280 × 800、1024 × 680；键盘 / 无横溢出、分区完整筛选、指定 portrait 文件、分页外旧排队任务、聚焦任务后作者全部作品、同资产重开焦点、失败 ASR 所属资料、已删除任务反馈、导入入口 / 取消、最近访问及刷新不计访问、错误保留 / 重试、文稿变更移出待办、真实空态；4 组结果 |
| `tests/library-writing.browser.js` | PASSED，5 组，三个视口原文阅读、编辑 / 笔记及未保存导航回归 |
| `tests/library-writing-failures.browser.js` | PASSED，9 组，保存失败、修订冲突、回读恢复、异步读取 / 提交、ASR 更新和文件变化 |
| scoped `rustfmt --check --config skip_children=true`、文档链接 / 全部依赖 DAG / 新旧文件空白检查、`git diff --check` | PASSED，任务 19 / 49、全部 49 项依赖引用存在且无环；Git 仅提示两个前批文件的 LF / CRLF 转换 |

浏览器测试仅使用 `?detail=1&writing=1&workbench=1` 合成资产和操作。工作台共享 fixture 的可变文稿 / 分类状态；其页面重载不是 Tauri 重启。真实持久化通过临时 SQLite 关闭重开验证，导入探测 / 取消通过离线 Core 与 Python 引擎 fixture 验证，不计为真实模型推理或真实平台下载。

调试中修正了开发模式重复挂载时合成订阅的回收问题，并保留“保存整理稿后自动离开待整理列表”的断言。测试适配 1024 分类栏折叠、合成 Task 必填字段、空库详情种子和导入 / 关联取消 mock 的组合；未降低生产校验或用默认成功代替失败。用户文案错误统一沿既有错误码映射，任务已移除时明确反馈。

### 审查与平台边界

按 `code-delivery-review` 进行只读子 Agent 审查。后端 checkpoint 已通过，12 个文件相对于本批基线完整覆盖，目标 SHA-256 `0891956065ef9d7df45a2b00ab187c2d7f650918ece95fc3a960c98fd7362d67`。最终审查为 APPROVED；Review mode 为 CHILD_AGENT，策略为 PARTITIONED_PLUS_INTEGRATION，后端、前端及集成均由同一独立审查 Agent 完成，Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH；1 轮修复后无剩余 P0 / P1 / P2。最终目标 SHA-256 为 `fb63703b887b2864891fd7233b799ba5421bba341af96c67a06e2e1a2dcf34e3`，覆盖本批 24 个代码 / fixture / 测试文件，含未跟踪 HTML 样本，独立复算一致。审查后仅回填本段与验证结果，代码未改变。审查发现聚焦下载任务后进入作者搜索时仍受单任务过滤，已清除聚焦状态并补充同作者其他作品可见断言；自查补充同资产从工作台重新进入时的标题焦点恢复，三个视口均验证。

Windows 浏览器与临时 SQLite 验证通过。真实 Tauri 工作台 / 原生导入选择器、真实 ASR、安装包及 Windows 125% / 150% 缩放人工验收为 `NOT_RUN`；macOS Intel / Apple Silicon 为 `UNAVAILABLE`。本批没有重复未受影响的 Python 和全仓库测试，也不宣称 P1A 阶段验收完成。


## 2026-10-27：导出备份与空库恢复核心

对应 WB-117。起始仍为 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`；保留此前七批未提交成果，按本批 377 文件基线区分改动。未提交、推送、安装新工具或访问用户真实数据库。任务清单为 20 / 49，WB-118 原生页面与选择器、WB-119 阶段验收及 MCP 保持未完成。

### 实现与复用

- `library/content_export.rs` 提供文稿 / 笔记 TXT、Markdown、JSON，Core `export_library_content` 按资产所属 ID 读取已保存内容。Markdown / JSON 带资产、版本、模型、逐版原文来源及跨资产引用元数据；JSON 为无 BOM 的 UTF-8，TXT 复用旧编码、平台换行、临时文件提交与媒体保护。未改前端旧 TXT 命令和 UI。
- `library/backup.rs` 白名单捕获知识库资料，目录包 v1 保存 manifest、清单 SHA-256、封面和可选视频；不含设置、凭据、下载任务 / 关联或运行摘要。ID、版本、revision、引用、分类树、标签、归档和最近访问可恢复；缺媒体仍可读写资料，已变化 / 重新关联的原文来源不冒充当前媒体。
- `library/transfer.rs` 复用标准库、现有 tempfile 与 sha2，64 KiB 流式复制与摘要，避免将视频整体读入内存；文件工作不持有 Core 状态锁，提供真实处理计数 / 字节和可取消回调。目标必须是新目录，包内名字由编号生成，恢复视频扩展名使用白名单。
- 恢复先以隔离内存 SQLite 校验当前 schema、关系归属、稳定 ID、循环和引用，再以目标同级暂存准备文件，在原 Core 空库事务写入。准备前与提交前二次检查空库和活动操作；发布后的 SQL 提交失败会回滚并清理刚创建的目录，成功才发一次整体资产变更事件。明确的清理失败单独报错。

已检查仓库现有文件提交、TXT 导出、SHA-256 与临时目录能力，并检索 [zip-rs/zip2](https://github.com/zip-rs/zip2) 的归档方案。本次契约不要求压缩，选择可直接检查与复制的目录包，不增加归档依赖或自行实现压缩 / 解压器；具体布局、上限、原文来源映射和错误见 [API 第 6 节](workbench-knowledge-base-api.md#6-单篇导出与逻辑包核心wb-117)。没有修改 SQLite schema 版本。

### 实际验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml library -- --test-threads=1` | PASSED，39 项；在原 29 项上新增 7 项逻辑包测试和 3 项 Core / 导出测试 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` | PASSED，172 项通过、0 失败、2 项原有在线 GitHub 检查 / 安装包下载测试按原配置 ignored，耗时 141.61s；doc-tests 0 项 |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASSED，包含现有 Tauri 入口对 TXT 导出共享函数的编译检查 |
| `cargo doc --locked --manifest-path src-tauri/Cargo.toml --no-deps` | PASSED，无文档诊断 |
| scoped `rustfmt --check --config skip_children=true` | PASSED，检查本批实现及测试；不格式化无关历史文件 |
| 文档引用、任务数量 / 依赖 DAG、新建与修改文件空白、`git diff --check` | PASSED；任务 20 / 49，49 项依赖无环；Git 仅提示前批两个文件的 LF / CRLF 转换 |

新增验证使用真实临时文件和 SQLite，不依赖网络或用户媒体：

- 两种模式恢复全部旧版本、AI 笔记来源、精确引文、父子分类、标签、封面、归档、最近访问；原视频删除后仍从含媒体包读回字节，仅资料包恢复为缺失占位。Core 关闭并重新打开临时 SQLite 后资料与访问仍保留，目标设置不变，也不伪造下载历史。
- 单篇三格式可读，JSON 直接由 serde_json 解析、TXT 有 BOM，错误所属 ID 不创建文件；新入口保护其他资产引用媒体及硬链接，导出不增文稿版本、笔记 revision 或成功事件。recognized → edited v1 → edited v2 的独立 Markdown / JSON 可辨识旧模型、文件来源及原文版本。
- 13 类坏包覆盖未来版本、摘要损坏、payload 损坏 / 缺失、`..`、盘符、反斜杠、NTFS ADS、重复路径、分类循环、错误归属 / 外键及伪造原句；六类实体空 ID 另有回归。坏包不发布目标目录或半成品资料。
- Windows 通过临时 NTFS junction 实际将 payload 目录指向包外，恢复明确拒绝；测试移除的仅为该临时联接，外侧合成 payload 保留。恢复文件名不受原文件名中的路径或可执行后缀控制。
- 复制中取消、复制中源文件变化、恢复中取消、发布后延迟外键提交失败均回收暂存和新输出；已有目录 / 用户文件、只有标签的非空库，以及准备后新增标签均拒绝覆盖。

实现调试中修正了测试对领域函数名称的假定，以及 Windows 扩展路径前缀与普通路径比较造成的误报；路径断言改为与规范化目标比较，未弱化包内路径检查。独立审查发现整理稿独立导出未附完整来源链，以及空 ID 可穿过 SQLite 约束却破坏 UI 前提，两项均已修复并补回归。

### 审查与平台边界

代码 checkpoint 已由独立 CHILD_AGENT 完整审查并通过，目标 `79803cc7e9975b2c86ec030f5580cfdf22840bd44f96a1a2f725f6e8f96dff48`，覆盖本批 12 个代码 / 测试文件；两个 P2 已解决，无剩余 P0 / P1 / P2。最终交付审查为 APPROVED，Review mode 为 CHILD_AGENT，策略为 FULL_SCOPE（沿用完整实现 checkpoint 并完成最终文档 / 集成复核），Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH；1 轮修复后无剩余 P0 / P1 / P2。12 个代码 / 测试文件和 7 份文档均完整覆盖，最终目标与上述 checkpoint 相同且独立复算一致；审查后仅回填本段结论，代码未改变。

Windows 临时文件、真实 SQLite、字节内容、SHA-256、事务失败和 junction 验证通过；备份 / 恢复核心尚未接 Tauri 新异步任务、选择器与页面，本批没有 UI 改动，也未重复浏览器验收。原生 GUI、真实大视频跨盘 / 断电或强制退出、安装包为 `NOT_RUN`；macOS Intel / Apple Silicon 为 `UNAVAILABLE`。Core 的关闭状态检查与正常错误 / 取消回收，不代表已验证应用进程被强制终止的文件清理协议。


## 2026-10-27：导出备份恢复界面与原生命令

对应 WB-118。起始 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，按本批 382 文件基线保留此前八批所有未提交工作。未提交、推送、安装新依赖或访问日常资料库。任务清单为 21 / 49；WB-119 阶段 / 原生验收与 MCP 保持未完成。

### 界面与核心集成

- `library-writing.tsx` 提供 TXT / Markdown / JSON 格式、文稿与笔记导出，统一按所属资产 / 内容 ID 调用新命令，未保存时禁用导出。选择器等待时禁止编辑、切换版本或通过引用创建新笔记；成功位置持续展示，错误与取消有反馈。旧 TXT 公共命令保留兼容。
- 设置新增知识库分组，`library-backup-settings.tsx` 默认仅资料备份，显示真实资产 / 文稿 / 笔记范围，包含视频时显示可用文件数与当前文件字节体量。已有资料 / 活动任务阻止恢复，取消目录选择不开始复制；升级前本机数据库备份与可迁移逻辑包的用途在按需说明中区分。
- `library_transfer_commands.rs` 六条命令接通另存为、备份父目录、恢复包目录与恢复父目录。输出子目录由 Rust 生成时间 / UUID 名，不接受前端任意目标路径。内容格式对应扩展名和过滤器；选择取消返回 null。
- `library_transfer_operation.rs` 在既有 Core worker 生命周期中管理单一备份 / 恢复操作，spawn_blocking 复制不占 UI 执行路径。真实阶段 / 计数 / 字节通过事件发送，64 KiB 检查取消，进度发送按阶段 / 文件变化或 100ms 节流；退出会取消并等待实际复制 worker 回收。取消接受与进入提交阶段使用同一 Core 临界区。
- `use-library-transfer.ts` 在应用根部保留状态，按 order / sequence 合并事件、启动返回与查询。切页不重复启动，工作台运行 / 失败入口可回到知识库设置；状态读取失败与订阅失败分别保持可重试反馈。操作的目标位置在运行、失败和完成状态均可见，清理失败时也可定位残留目录。

### 实际验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml library -- --test-threads=1` | PASSED，41 项；新后台操作成功 / 失败 / 恢复，以及受控暂停后取消 / 退出等待真实复制、暂存回收。窄取消修复后相关 2 项再次通过 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` | PASSED，174 通过、0 失败、2 原有 GitHub live 检查 / 安装包下载测试按原配置 ignored，143.82s；doc-tests 0 项 |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASSED，包含六条新原生命令及事件注册 |
| `npm run build` | PASSED，TypeScript 与 Vite；既有 use-client / sourcemap 与体积提示保留 |
| `tests/library-backup.browser.js` | PASSED，4 组。1440 × 900 / 1280 × 800 / 1024 × 680：范围默认、键盘切换、体量、取消选择、真实摘要进度、工作台返回、提交禁取消、长路径与结果；另覆盖恢复失败重试 / 列表刷新、取消、旧操作高 sequence 不覆盖新操作、订阅断开不被读取成功掩盖、三格式 / 笔记、等待导出时引用禁用与未保存禁导出 |
| `tests/library-writing.browser.js` / `tests/library-writing-failures.browser.js` | PASSED，5 / 9 组，三个视口编辑 / 来源 / 引用 / 冲突 / 未保存保护和旧 TXT 取消 / 错误回归 |
| `tests/workbench.browser.js` | PASSED，4 组，旧资料入口 / 访问、任务聚焦、作者与导入取消回归 |
| scoped rustfmt、文档引用、任务依赖 DAG、新建与修改文件空白、`git diff --check` | PASSED；21 / 49，49 项依赖无环，WB-119 保留待验收；Git 仅有前批两个文件的换行提示 |

浏览器使用 `?detail=1&writing=1&backup=1` 的合成资产、独立 IPC mock 和可控事件；原生选择器取消、保存路径及进度是 UI 契约验证，不能证明实际 OS 对话框。文件内容、SHA-256、关系、失败事务和暂存回收沿用 WB-117 的真实临时文件 / SQLite 测试，并新增真实 1 MiB Core worker 的取消和退出等待。没有模拟百分比或演示资料进入生产界面。

调试中修正了 fixture 的路径转义及 textarea 名称，并在键盘测试等待 Radix 异步焦点转移后再按 Enter，保留真正键盘激活断言。测试暂停器最初把通知锁持有到等待释放期间，已在等待前释放该测试锁；进度很快时只有阶段变化携带完整字节，因此断言检查真实字节而不要求短复制一定存在中间 copying 百分比。

### 原生验证边界

用临时 Cargo example 搭建隔离 Tauri harness，直接复用本批生产 `library_transfer_commands.rs`，指定独立标识、资料路径和 WebView 数据目录；源码及日志保留在 `.local/wb118-native/harness/`、`.local/workbench-batch9-native-build.log` / `native-run.log`。其编译成功，但进程在 main 之前以 `0xc0000139 / STATUS_ENTRYPOINT_NOT_FOUND` 退出，实际窗口未出现，测试资料库也未创建；真实原生另存为 / 目录选择 / 取消 GUI 验收为 `UNAVAILABLE`，没有以编译或 IPC mock 冒充通过。

临时 harness 的三个仓库源文件已归档到本批 `.local` 证据并仅删除本次创建的这三个文件；未修改生产应用标识或增加测试后门。没有为解决宿主加载器问题扩展依赖或系统设置。Windows 安装包、真实大视频跨盘、缩放、原生强杀仍为 `NOT_RUN`；macOS Intel / Apple Silicon 为 `UNAVAILABLE`。本批交付实现与离线 / 浏览器验证，整阶段原生验收保持 WB-119 未完成。

### 独立审查

后端 6 文件 checkpoint 已通过 CHILD_AGENT 只读审查，目标 `e107dd55c25b8eee14444c3454e3a341834750b0ed72baea02353a031d9f963c`。发现并修复提交前取消检查交错的 P2；前端审查发现并修复导出等待时引用动作可切换编辑目标、失败 / 运行操作未显示目标位置的两项 P2，均补充边界断言。最终交付审查为 APPROVED，Review mode 为 CHILD_AGENT，策略为 PARTITIONED_PLUS_INTEGRATION（后端 6 文件与前端 12 文件均完整覆盖，再复核跨层契约和 8 份文档），Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH；2 轮窄修复后无剩余 P0 / P1 / P2。最终目标 SHA-256 为 `2c94cfbc3d5e173b924a7c93db740b9991329b14eba0a8cebb4318307406a1a9`，独立复算一致；审查后仅回填本段，代码未改变。

## 2026-10-27：P1A 集成与 Windows 原生验收

对应 WB-119 的 Windows 部分。保留 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0` 和此前全部未提交成果，以 `.local/workbench-batch10-baseline/` 的 389 文件清单划定本批差异。没有提交、推送、安装工具、访问真实用户库或使用平台凭据。本轮用户确认先完成 Windows，macOS Intel / Apple Silicon 保留待验收；WB-119 总项仍为未完成，进度仍是 21 / 49。

### 启动错误与验收入口

第九批临时 Cargo example 的 `0xc0000139` 已定位：它导入 `comctl32!TaskDialogIndirect`，但没有嵌入 Common Controls v6 manifest。Tauri 构建脚本生成的 Windows `resource.lib` 默认只链接到 bins，example 不会自动继承；它缺少的还有生产托盘所需图标。只补 manifest 虽能越过加载器，随后仍会因图标缺失退出。Common Controls v6 要求应用声明激活上下文，见 [Microsoft 文档](https://learn.microsoft.com/en-us/windows/win32/controls/common-control-versions)。

本批在 `build.rs` 的 desktop + Windows MSVC 条件下，把同一 `resource.lib` 链接给 examples，复用现有 manifest、图标和版本资源。新 example 和正常生产 bin 均确认有 RT_ICON / RT_GROUP_ICON / RT_VERSION / RT_MANIFEST（资源类型 3 / 14 / 16 / 24），生产 bin 编译通过。此前失败的对象是临时验收 example，不能据此判定正式应用也有相同启动故障；本次没有替换系统 DLL 或更改系统设置。

新增 `scripts/p1a-acceptance.ps1`、`examples/p1a_acceptance.rs` 和 `tests/fixtures/seed_p1a_profile.py`：

- 复用完整生产界面、IPC、Core、文件系统和引擎，没有备用 UI 或 mock 命令。生产 `main` 仍使用默认 Builder / Context，公共 `run` 只让 example 注入隔离检查。
- 独立编译标识、资料目录和 EXE，预建空库或合成旧库。实际 Context 必须匹配编译配置；隔离插件在生产 setup 之前检查实际 app_data_dir 的标记，防止触发生产旧目录迁移。
- 直接启动 EXE 时临时移走其标记，进程以 `PluginInitialization("p1a-isolation", missing file)` 拒绝进入生产 setup；恢复标记后同一 EXE 可重启。runner 拒绝复用未标记 / 不匹配的目录，以无 BOM UTF-8 写标记。
- 重跑同一场景不重建数据库或改写已有视频指纹。合成旧作品使用合法数字 ID、图集使用 dtk，Windows 原文来源使用 Rust canonicalize 的路径形式，避免把 fixture 构造错误当成产品迁移失败。

启动时还确认本机原先的 PyInstaller sidecar 早于新增本地探测协议：实际导入返回持续可见的引擎错误。按现有流程重建 sidecar 后，同一原生导入成功；打包引擎握手为 0.15.0 / yt-dlp 2026.08.19。源码开发须同时更新 sidecar，具体命令已写入 [开发文档](development.md#windows-p1a-隔离原生验收)。

### Windows 原生流程

以下均在本机 Windows x64 的真实 Tauri WebView2 窗口执行。界面来自 Vite，Rust 为 desktop debug 构建，Python 为实际打包引擎；资料均为合成数据。此处不代表安装包验收。

| 场景 | 实际结果 |
| --- | --- |
| 空库启动与导入 | `wb119/empty` 初始列表为空；原生 Windows 文件选择器导入中文路径 `待导入.mp4`，SQLite 为 1 资产 / 1 文件，不复制原视频 |
| 播放与详情 | asset 协议成功读取 H.264 样本，画面播放到 0:04 / 0:04；分辨率 90 × 160、文件大小和可用状态与源文件一致 |
| 整理稿与笔记 | 原生保存两段中文、`100%_` 和 emoji 整理稿，以及制作手法笔记；数据库分别为 edited v1、human r1，未使用真实 ASR 模型生成原文 |
| 原生另存为 | 导出整理稿 TXT 到独立证据目录；核对 UTF-8 BOM、CRLF、完整两段正文和 emoji，界面持续显示导出路径 |
| 含视频备份 | 1 资产 / 1 文件，0 文件未提供；包内 manifest、校验文件、1 个媒体 payload 与界面结果一致 |
| 空库恢复 | `wb119/restore` 通过两个原生目录选择器恢复上述包；1 资产 / 1 篇文案 / 1 条笔记，列表立即刷新，非空库恢复入口禁用 |
| 恢复一致性 | SQLite 的资产、正文、笔记、ID、最近访问与原库一致；恢复媒体 SHA-256 与原视频一致，设置仍使用恢复场景自己的目录 |
| 多分类与标签 | 原生新建“文案学习”“制作手法”和“节奏”，同一资产加入两个分类及一个标签；资产 / 文件仍各 1 个；重启后关系、文案和笔记保留 |
| 中文检索与工作台 | 搜索笔记独有短词“短镜头”命中并显示笔记摘要；重启后工作台最近访问 1、待分类 0、待整理 0、运行中 0，与实际数据一致 |
| 混合旧库升级 | 5 条历史含同作品同路径重复、另一文件版本、缺失视频和图集，迁移为 2 资产 / 3 文件 / 2 原文，生成 1 份升级前数据库备份；图集仍留在下载历史 |
| 缺失源文件 | 列表准确显示缺失，播放入口禁用；详情仍可阅读旧模型原文，来源变化提示可见 |
| 升级后重启 | 停止无活动操作的隔离进程后重启，资产 / 文件 / 文案整行比对相同，无重复迁移 |
| 取消与默认仅资料备份 | 原生目录选择器按 Escape 取消后显示“已取消选择”，未创建包；再执行默认范围，得到 2 资产 / 2 原文、0 视频 payload，3 文件未随包提供，包不含设置 / 凭据 |

证据目录为 `.local/p1a-acceptance/wb119/{empty,restore}/`、`.local/p1a-acceptance/wb119-r1/upgrade/` 和最终规范路径 fixture 的 `wb119-final/upgrade/`；包含数据库、原视频、TXT、逻辑包、独立 EXE 与 stdout / stderr。整行及媒体比较见 `.local/workbench-batch10-restore-comparison.json`、`restart-counts.json`、`upgrade-counts.json`、`upgrade-restart.json`；这些是本次本机证据，不随代码分发。重启使用停止自有隔离进程，无活动任务；不将其称为托盘正常退出验收。

### 自动回归与阶段要求映射

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1` | PASSED，174 项，2 项既有 GitHub 在线测试 ignored；本批新增测试随后单独验证，不冒充全量已重跑 |
| 新增 `p1a_upgrade_learning_history_clear_and_material_restore_round_trip` | PASSED；实际 Python 造旧库 → Core 迁移 → 人工稿 / 引用笔记 / 多分类标签 → 重启 → Core 清空历史 → 删除自有视频后标缺失 → 仅资料备份 → 空 Core 恢复；版本、ID、分类及引用一致 |
| `.venv/Scripts/python.exe -m pytest -q` | PASSED，463 项，含本地探测、协议与现有引擎回归 |
| `npm run build` | PASSED，类型与 Vite 构建通过；仅现有 chunk 体积提示 |
| desktop `cargo clippy --locked --all-targets -- -D warnings` | PASSED，含生产 bin 与验收 example |
| 正常生产 `yingqu` desktop bin | 编译 PASSED，manifest / 图标资源核对 PASSED；未直接启动日常标识，以免访问用户资料库 |
| PyInstaller 重建与 `scripts/check_packaged_engine.py` | PASSED，0.15.0 握手与当前 sidecar 本地导入通过 |
| 知识库浏览器回归 | 8 份脚本全部 PASSED；列表 7 组、边界 3 组、详情 6 组、播放 7 组、编辑 5 组、编辑失败 9 组、工作台 4 组、备份恢复 4 组；覆盖 1440 × 900、1280 × 800、1024 × 680 |
| 中文全文规模复测 | 1000 资产 / 1000 文档 / 16,218,000 UTF-8 字节，3 次查询 103 / 102 / 100 ms，返回首屏 20 项；SQLite 内存库 + 本机 debug 构建的合成基准，不推断真实大库磁盘延迟 |

浏览器订阅断言已适配真实工作台：下载页为 0 个知识库订阅，工作台自身为 1 个，返回知识库仍为 1 个，继续验证旧页面释放与无重复订阅。没有弱化为仅检查“页面能打开”。日志统一为 `.local/workbench-batch10-*.log`。

方案第 11 节的六项要求分别由以下证据承接：

1. 多分类、后代筛选、批量事务和重启：本次原生分类归属 / 重启、新集成测试及 `root_names_cycles_multi_membership_and_atomic_batches`。
2. 历史清理与缺失文件：新集成测试调用真实 `Core.clear_history`，旧 `task_removal_keeps_documents_notes_categories_and_legacy_cover` 验证封面保留，文件删除 Core 测试和本次缺失详情验证文本仍可读。
3. 混合迁移、幂等和失败原子性：本次升级 / 重启、新集成测试、`migration_merges_identity_preserves_versions_and_skips_non_videos` 与 `migration_error_rolls_back_all_assets_and_can_be_retried`。
4. 编辑 / 重新识别 / 失败取消 / 重新关联：当前 Rust 转录生命周期与资源回收测试、编辑失败浏览器 9 组，以及此前已记录的源指纹 / 文案版本测试；原生本轮新增人工稿保存与缺失原文阅读。真实模型准确率不作为这些离线结果的推论。
5. 中文、特殊字符、正文笔记、筛选分页及规模：当前 Rust search 测试、三视口列表 / 编辑回归、本次原生笔记短词查询和上述规模数据。
6. 无私密数据的逻辑包、空库一致恢复与缺媒体：本次两个范围原生备份 / 原生空库恢复、新集成测试及现有 backup 完整性、取消和来源引用测试。

### 平台结论与剩余验收

Windows 源码开发形态的 P1A 集成和上述原生流程为 PASSED。macOS Intel 与 Apple Silicon 为 `UNAVAILABLE`，按用户本轮决定继续待验收，WB-119 总项不勾选。Windows 安装包 / 全新设备、125% / 150% 缩放、真实平台在线下载、真实 ASR 模型推理、托盘正常退出与长时间自然媒体视听本轮为 `NOT_RUN`；保留各自专项验证边界，不以浏览器或合成数据冒充。尚未实施 MCP，版本号和发布状态不变。

### 独立审查

本轮 Windows 限定交付审查为 APPROVED，Review mode 为 CHILD_AGENT，策略为 PARTITIONED_PLUS_INTEGRATION。完整覆盖 8 个代码 / 测试文件、3 份文档及跨层证据；Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2。1 轮 P1 修复补全隔离目录标记检查，缺标记启动失败证据已闭环；marker BOM 兼容问题及合成来源路径误差亦已修正。最终代码映射 SHA-256 为 `1774a47963d599b569a3410daa7e1b9984f016e529d236208214dadde0de4f6c`，逐文件清单见 `.local/workbench-batch10-code-r2.json`（清单文件 SHA-256 `5b7ded3a314356584b2bbe9eb039f3541c13fb67859efa24fc865be5899fdebc`），审查者独立复算一致。审查后仅回填本段元信息，产品代码与验收结论未变；此结果不代表三平台 WB-119 整体完成。


## 2026-10-28：MCP 协议与技术选型验证

对应 WB-201 的技术选型与 Windows 部分。起始仍为 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，以 `.local/workbench-batch11-baseline/` 的 393 文件清单保留此前全部未提交成果。没有提交、推送、安装新工具、改动真实客户端配置或访问真实资料库。本轮明确将 macOS 验收继续保留待办。

### 交付与选型

选定官方 `rmcp = 3.8.0`，crate 声明 Apache-2.0、最低 Rust 1.88；SDK 负责 MCP 全协议和 stdio，不自行实现生产协议栈。独立 `tests/mcp-sdk-probe/` 仅提供合成 echo 工具，拥有独立 Cargo.lock；未将 SDK 加入生产依赖。目标本机客户端为已安装的 Codex CLI 0.162.1。官方版本、完整许可证、两种协议生命周期、后续 Core 适配边界和平台状态见 [MCP 选型验证](mcp-sdk-validation.md)。

新增 `scripts/verify-mcp-sdk.mjs`，复用项目已安装、package-lock 锁定的官方 TS SDK 1.32.1。运行器复制 probe 到含中文和空格的目录，通过真实子进程验证旧式握手与现代 discovery；Codex 分支使用独立 home、配置和 cwd，只进行工具发现，不创建对话或模型 turn。真实媒体、生产应用数据和客户端日常配置均不参与该样例。

### 本轮验证

| 验证 | 结果 |
| --- | --- |
| 独立 Cargo build / Clippy / fmt | PASSED；`--locked` 构建及 `--all-targets -- -D warnings` 通过；Node 语法检查通过 |
| 官方 TS SDK → rmcp stdio | PASSED；实际协商 2025-11-25，工具发现、调用、Schema 2020-12、文本与 structuredContent 一致 |
| 现代 MCP | PASSED；2026-07-28 discovery → tools/list → tools/call，无 initialize；未知版本 -32022，缺必要 metadata -32602 |
| 错误与 Unicode | PASSED；类型错误 isError=true、未知工具 RPC -32602，中文 / emoji / 特殊字符 / 换行回显及含空格中文路径启动通过 |
| 退出 | PASSED；关闭 stdin 后 probe / Codex app-server 正常退出，现代 raw stdout 每行均为可解析 JSON，未强制终止 |
| 真实 Codex CLI 0.162.1 | PASSED；`mcpServerStatus/list` 经真实 MCP 客户端发现 probe_echo、服务信息、只读注解和输入 / 输出 Schema；模型总结为 NOT_RUN |
| 不带 `--codex` | PASSED；协议两路径执行，真实客户端项准确标记 NOT_RUN，未冒充客户端验收 |
| 生产边界 | 根 package.json / package-lock.json、生产 Cargo.toml / Cargo.lock 与基线完全一致；没有改 UI、Core、IPC 或 sidecar |

最终完整运行证据为 `.local/wb201/run-mYfU1S/`，含 summary、typescript / modern / codex 双向报文和 stderr；不带 Codex 的分支为 `run-Hxsa7Y`。构建和 feature 图在 `.local/wb201/probe-build.log`、`probe-clippy.log`、`probe-features.log`，官方版本元数据和本机 Codex Schema 亦在同目录留存。复验入口已写入 [开发文档](development.md#wb-201-mcp-sdk-验证)。本轮没有改动生产执行代码，未重复前批前端、下载和 ASR 全量测试，不将既有测试结果记为本轮重新执行。

### 任务与平台状态

技术选型、可运行最小样例、Windows 双模式互通及目标客户端发现已完成。macOS Intel / Apple Silicon 的编译、启动与目标客户端连接均为 `UNAVAILABLE`，按用户要求继续待办，因此 WB-201 总项保持未勾选；WB-119 的 macOS 待验收状态不变，仍为 21 / 49。

知识库 IPC、stdio 生产适配、实际资料工具 / Resources、设置开关、安装包和客户端模型总结没有在本轮交付，分别由 WB-202～WB-208 承接。echo 样例只能证明本次协议和环境的兼容性，不能证明 P1B 或产品可读取真实知识库。

### 独立审查

本轮技术选型与 Windows 限定交付审查为 APPROVED，Review mode 为 CHILD_AGENT，策略为 PARTITIONED_PLUS_INTEGRATION。代码四文件和六份文档完整覆盖，并核对实际线报文、客户端隔离、任务状态及后续接入边界；Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2。审查发现并修复 TS transport 合并默认环境带入真实用户目录的 P2，重跑完整与无 Codex 两分支通过。最终代码目标为 `e40b7e57754f83863800b127274f81b1788370c0fbf7b911a27350aeae4235fa`，清单见 `.local/wb201/code-final.json`；完整交付清单见 `.local/wb201/final-target.json`，均由审查者独立复算。审查后仅回填本段元信息，不改变代码和验收结论；macOS 仍待验收，本结论不代表 WB-201 全平台或 P1B 整体完成。


## 2026-10-28：应用 Core 本机 IPC 与 Windows 命名管道

对应 WB-202。起始分支 `master`、HEAD `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，以 `.local/workbench-batch12-baseline/` 的 397 文件清单和源码快照划定本批差异，保留之前全部未提交工作。没有提交、推送、安装新工具、改动真实 AI 客户端配置或读取日常应用数据库。

### 交付

新增 `core_ipc.rs`、平台 transport、`mcp_ipc_api.rs` 及原生进程集成测试，Core 持有可选监听器并默认关闭；桌面注册状态与启停命令。监听器和连接使用同一 Core，调用现有分类 / 标签读取，不新增数据库连接或调度器，不启动引擎。关闭时等待监听和所有会话回收，Core shutdown 禁止再次开启；重启保持关闭。

Windows 复用已锁定 Tokio 1.53.2 命名管道，通过原生 SID / SDDL API 设置只有当前用户的受保护 DACL，拒绝远程客户端，首实例防止覆盖已有端点；仅增加现有 windows-sys 的 `Win32_Security_Authorization` feature，没有依赖版本或 Cargo.lock 变化。macOS Unix socket 使用 `/tmp/yingqu-mcp-<uid>/` 短路径、当前 UID、0700 目录及 0600 socket，避免资料库或系统临时目录过长超出 sockaddr_un 限制；权限、失效 socket 回收与清理的实现 / 测试已落盘，当前没有 Mac 编译和执行证据。

内部协议 v1 明确只读白名单 `ping` / `list_categories` / `list_tags`，有请求 ID、版本、字段与参数校验、请求 / 响应大小限制，以及独立会话关闭处理；非法请求不执行写入，不暴露 SQL、设置、凭据或内部文件元数据。端点、协议、错误码、启停边界与后续任务划分见 [本机 IPC 契约](mcp-local-ipc.md)。

### 本轮验证

| 验证 | 结果 |
| --- | --- |
| Windows 定向单元测试 | PASSED；6 项覆盖实际命名管道及 DACL、实时 Core 分类 / 标签、默认关闭、重复启停、端点占用失败、资料库隔离、非法请求、Unicode 分片、大小限制和连接回收 |
| 原生跨进程测试 | PASSED；1 项父测试实际启动独立子进程 Core，在含中文空格的临时 profile 中读取合成分类，随后经 Core shutdown 退出，客户端收到断连且无法重连；ignored 的子进程 fixture 已由父测试以 --ignored --exact 执行 |
| Core 全量回归 | PASSED；180 通过、2 项既有 GitHub 在线测试忽略；随后补充的端点占用断言由上述 6 项定向测试覆盖，不把未执行在线项计为通过 |
| 桌面 Rust Clippy | PASSED；`--locked --features desktop --all-targets -- -D warnings`，包含生产命令与既有隔离验收 example |
| Rust 格式 / 文档 | PASSED；新增 Rust 文件 rustfmt 检查、作用域文档本地链接和差异空白检查通过 |
| Windows GUI / 安装包 / AI 总结 | NOT_RUN；本批不改 UI、不构建安装包，不交付 stdio 适配与模型调用；跨进程 Core 结果不替代完整原生 GUI 验收 |
| macOS Intel / Apple Silicon | UNAVAILABLE；无可用机器，编译、Unix socket 权限及原生生命周期验收继续待办，不用 Windows 结果替代 |

命令见 [开发文档](development.md#wb-202-本机-ipc-验证)。证据为 `.local/wb202/ipc-unit.log`、`ipc-cross-process.log`、`core-regression.log`、`desktop-clippy.log`，仅包含测试输出与合成数据。实际管道 DACL 检查证明唯一允许 ACE 与当前用户 SID，未切换另一个真实用户账户；这是权限结构验证，不是跨账户登录验收。

### 任务状态

WB-202 的 Core 服务、Windows 命名管道及验证子项完成，两种 Mac 继续保留待办，WB-202 总项不勾选；WB-119 / WB-201 的既有 macOS 待验收状态不变，主任务仍为 21 / 49。完整资料工具、公开 DTO / Resources、stdio 生产适配、产品开关界面、客户端配置及分发仍在 WB-203～WB-208，不将 IPC 测试描述成 AI 已能读取知识库。


### 独立审查

本批 Windows 限定交付为 APPROVED，Review mode 为 CHILD_AGENT，策略为 PARTITIONED_PLUS_INTEGRATION。完整覆盖 10 个代码 / 测试 / feature 文件、7 份文档增量及跨层关系，Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2。审查发现并修正了 macOS 普通临时目录可能超过 Unix socket 路径长度限制的问题：改用固定 UID 私有短目录，并补充长 Unicode profile 及端点隔离测试；Unix 源码审查不代替原生执行，Mac 状态仍为 UNAVAILABLE。最终代码清单见 `.local/wb202/code-target.json`，SHA-256 身份为 `0b21262d658b48ad6e3c1938ffefc6f1dee7b07e2dd58b281969ff440c0448f6`；完整交付清单见 `.local/wb202/final-target.json`。审查后只回填本段元信息并复核，不改变产品代码或平台验收结论。


## 2026-10-28：stdio MCP 适配进程与 Core IPC 映射

对应 WB-203 的实现与 Windows 部分。起始仍为 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，以 `.local/workbench-batch13-baseline/` 的 404 文件清单 / 源码快照保存之前全部未提交成果。本轮未提交、推送、安装工具、修改日常 AI 客户端配置或访问真实资料库。

### 交付与边界

生产增加可选 Cargo feature `mcp`，固定官方 rmcp 3.8.0，只启用 server / macros / transport-io；独立入口 `yingqu-mcp --endpoint <系统端点>`。Cargo.lock 只新增 rmcp、rmcp-macros、futures、pastey、schemars_derive、serde_derive_internals 六个包，所有起始已有包的版本和集合保持。初次离线解析曾选择无关降级，已恢复本批基线锁文件并正常解析；最终清单未保留任何该类降级。

适配进程先连接运行中 Core、验证 ping，再交由 SDK 完成 MCP 握手。只映射现有分类 / 标签读取，不直接开库、不启动应用、不开启 Core IPC、不隐式重连，也不创建其他子进程。每条 IPC 交换有序并校验版本、ID、结构和大小，支持空闲断连监督和 5 秒超时。stdout 只承载 SDK 协议，错误仅以固定代码写 stderr / SDK 错误；客户端保持 stdin 打开时，Core 关闭仍能取消 SDK 并退出。

新增 `scripts/verify-mcp-adapter.mjs` 和仅供测试的 `examples/mcp-ipc-fixture.rs`，以标记文件、新建临时资料库、精简环境及隔离 home 运行。包含中文和空格的真实可执行路径、分类 / 标签名，以及实时 Core 修改，不使用真实媒体或凭据。启动、协议与错误详见 [stdio 适配进程契约](mcp-stdio-adapter.md)。

### 本轮验证

| 验证 | 结果 |
| --- | --- |
| 可选 feature 构建 | PASSED；`--locked --features mcp --bin yingqu-mcp --example mcp-ipc-fixture`，不依赖桌面窗口入口 |
| Adapter IPC 单元测试 | PASSED；4 项覆盖并发对应、空闲断连、5 秒超时与队列失败、错误脱敏、版本 / ID / 互斥 / 结构 / 大小及截断响应、非法端点 |
| 官方 TS SDK 1.32.1 → 真实 adapter | PASSED；旧 2025-11-25 initialize / initialized、两项工具 Schema / 只读注解、并发调用、空参数 / 省略参数、中文 emoji、Core 实时改名和结果两种内容一致；额外参数 isError、未知工具 -32602 |
| 现代 MCP | PASSED；2026-07-28 discovery → tools/list → tools/call，无 initialize；不支持版本 -32022、缺 metadata -32602 |
| 客户端关闭 / EOF | PASSED；握手前后关闭 stdin 均自然退出 0，Core 保持运行；实际 stdout 每行均为 JSON-RPC，不用 kill 冒充正常结束 |
| 服务关闭与 Core 退出 | PASSED；旧 / 现代会话在 stdin 保持打开时自然退出 1，stderr 仅 IPC_DISCONNECTED；服务关闭或应用未运行时启动退出 1、仅 IPC_UNAVAILABLE，stdout 无握手成功响应；服务重新开启后新适配进程能重新读取 |
| CLI 与日志 | PASSED；缺参数退出 2，非法端点退出 1，不回显敏感示例端点，只写固定诊断到 stderr |
| Core 回归与原生 IPC | PASSED；含 mcp feature 的 Core 全量 185 通过、2 项既有 GitHub 在线测试忽略；原生跨进程 IPC 另 1 项通过，ignored 子入口由父测试实际执行 |
| 桌面全目标 Clippy | PASSED；`--locked --features desktop,mcp --all-targets -- -D warnings`。初次重复 AsyncRead supertrait 的 lint 已修正，初始日志保留 |
| 格式 / 文档 / 差异 | PASSED；新增 Rust 格式、Node 语法、作用域文档本地链接和差异空白检查通过 |
| macOS Intel / Apple Silicon | UNAVAILABLE；两种原生环境均不可用，编译、SDK / Unix socket 和自然进程回收继续待验收 |
| 模型总结 / 安装包 / GUI | NOT_RUN；完整工具、产品开关 / 配置界面、真实 AI 客户端组合验收及包装分发由后续任务实施 |

最终协议与进程证据位于 `.local/wb203/run-6ioyRK/`，含 summary、官方 TS 双向报文、旧 / 现代会话和各关闭 / 错误分支的 stdout 与 stderr；之前两次合成运行也保留。构建、单元、Core 回归、跨进程和 Clippy 日志在 `.local/wb203/` 对应文件；复验命令已写入 [开发文档](development.md#wb-203-stdio-适配进程验证)。本轮没有运行前端或 Python 全套检查，不把前批 UI / ASR / 安装包结果计为本轮执行。

### 任务状态

WB-203 的独立入口、SDK 握手、现有 IPC 工具映射及 Windows 服务关闭 / 断连 / 自然进程回收子项完成。macOS Intel / Apple Silicon 按用户要求继续待验收，总项不勾选；WB-119 / WB-201 / WB-202 同样保留 Mac 待办，主任务仍为 21 / 49。已有的两项读取映射不等于 WB-204 的完整七项工具或 WB-208 的 P1B 验收，设置界面和安装包保持后续范围。


### 独立审查

本批 Windows 限定交付为 APPROVED，Review mode 为 CHILD_AGENT，策略为 PARTITIONED_PLUS_INTEGRATION。完整覆盖 10 个代码 / 可执行文件、7 份文档增量及跨模块集成；Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2。已核对隔离环境、fixture 防误用、真实双向报文、退出码、stderr 和进程清理，无以强制终止冒充通过的情况。最终代码身份为 `0adc8ab23b293a92edf0513f4c96467741463876b030cf7370dc8ff45f681dd6`，清单见 `.local/wb203/code-final.json`；完整交付清单见 `.local/wb203/final-target.json`，审查者已独立复算。

另以本地 / 锁定的 Tauri CLI 2.11.4 对照[官方固定版本源码](https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.11.4/crates/tauri-cli/src/interface/rust.rs)确认未启用 required-features 的 bin 会被过滤，目录扫描不会加回，因此默认 desktop 构建不会收集 yingqu-mcp；这是源码核验，不是本轮实际打包通过。审查后仅回填本节元信息并复核，产品代码和验证边界不变；macOS 两种架构仍待原生验收。


## 2026-10-28：P1B 七项只读工具与领域服务适配

对应 WB-204 的实现和 Windows 部分。起始 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，以 `.local/workbench-batch14-baseline/` 的 411 文件清单及源码快照隔离本批差异，保留此前全部未提交工作。没有提交、推送、安装工具、修改用户客户端配置或访问日常资料库。

### 实现与契约

新增 `mcp_read/` 公开 DTO、只读领域适配和契约测试；Core 通过原有互斥锁及同一 SQLite 连接完成每次请求，不修改资料或访问记录，不另开库或启动引擎。SDK 七项工具完整提供 `search_assets`、`get_asset`、`read_transcript`、`list_categories`、`list_tags`、`list_notes`、`read_note`；经 IPC 明确的 mcp_* 方法分发，旧 ping / list_categories / list_tags 协议保持不变。

复用现有检索、资产详情、分类标签、文档和笔记领域服务；公开结果只选择必要字段，不返回内部路径、Source、canonical_url、指纹、设置或凭据。列表默认 20 / 最大 100，正文默认 4000 / 最大 16000 Unicode 标量；文档 ID / 版本与笔记修订号可固定，正文范围及引用保持真实来源。引用单独分页，每条 quote_excerpt 最多 120 标量，完整原句长度与偏移通过文档引用保留。

请求字段、游标、版本 / 修订冲突、归属和范围有明确错误；业务错误后仍可继续调用。适配器在写 IPC 前检查 64 KiB 上限，过大参数返回 INVALID_INPUT，不留下半帧。内部 IPC 响应上限为 1 MiB，超限明确返回 RESPONSE_TOO_LARGE；这个限制不等同于 SDK 外部 MCP 总封包字节数，SDK 的 structuredContent 与文本双表示可能放大封包。所有输入 / 输出 DTO 使用同一 Schema 定义，通过 Schemars 原生字段 transform 去掉默认 TS 验证器忽略的 uint32 / uint64 格式注解，保留整数、非负和上下限约束。

共享 `tests/fixtures/mcp_library.rs` 为 Core 定向测试和隔离原生 host 提供多资产、多版本、文件状态、人工 / AI 笔记及跨资产引用；`scripts/mcp-read-checks.mjs` 接入现有 stdio 运行器的 --library 模式。完整字段与分页 / 切片 / 引用语义见 [只读工具契约](mcp-read-tools.md)。没有新增依赖、库结构、UI 或打包变更。

### 本轮验证

| 验证 | 结果 |
| --- | --- |
| 公开读取 Core 测试 | PASSED；6 项，桌面搜索一致、稳定分页 / 计数、默认及最大范围、旧版本、revision 冲突、跨资产来源、Unicode / ZWJ / 组合字符 / NUL、未知 ID / 游标 / 归属、超限与边界、内部字段省略、长引用分页及限长 |
| 无 MCP SDK feature | PASSED；同一 6 项 Core 测试不启用 mcp feature 运行通过，领域适配不强制引入 SDK |
| Adapter IPC 测试 | PASSED；5 项，保留既有并发 / 空闲断连 / 超时与错误校验，新增超大参数写入前拒绝及领域错误映射 |
| 官方 TS SDK + 真实 stdio / Windows 管道 | PASSED；全部七工具的 Schema、成功读取、分页、文档版本 / 笔记修订、精确 Unicode 引用、错误后继续读取、未知写入 / 路径工具拒绝；默认和最大正文长度、超大请求及内部响应超限均有实际调用 |
| SDK 注解互通 | PASSED；公开 schema 没有 uint32 / uint64 格式警告，保留 JSON integer 和范围约束，不通过关闭客户端校验规避问题 |
| 生命周期与协议回归 | PASSED；--library 继续执行旧 initialize / 新 discovery、非法 metadata / 版本、EOF、Core 服务关闭 / 应用退出时 stdin 保持打开、重新开启后新适配进程连接及自然进程回收；不带 --library 的原有模式也通过 |
| Core 全量回归 | PASSED；192 通过、2 项既有 GitHub 在线测试忽略，不把在线项计为通过 |
| 原生 IPC 跨进程 | PASSED；1 项父测试通过，ignored 子入口由父进程实际调用，旧协议调用保持兼容 |
| build / Clippy | PASSED；--locked 构建适配器及 fixture；--features desktop,mcp --all-targets -- -D warnings 通过 |
| 格式 / 文档 / 差异 | PASSED；本批 Rust 格式、JS 语法、本地文档链接、差异空白及任务状态检查通过，生产 Cargo.toml / Cargo.lock 未变 |
| macOS Intel / Apple Silicon | UNAVAILABLE；无对应机器，编译、Unix socket 与七工具原生验收继续待办 |
| Resources / 模型总结 / GUI / 安装包 | NOT_RUN；不属于本批实现和验收，不把只读工具调用等同于 P1B 完整交付 |

最终完整 stdio 证据在 `.local/wb204/run-AJppyu/`，含 summary、官方 TS 双向报文、读取结果及旧 / 现代协议和进程关闭分支；原有不带 --library 模式在 `.local/wb203/run-4Runge/`。初期带非标准格式警告的通过运行保留在 run-2NKkcb / run-1t3EOP，不作为最终无警告证据。其他结果在 `.local/wb204/read-unit.log`、`read-without-sdk.log`、`adapter-regression.log`、`core-regression.log`、`ipc-regression.log`、`build.log`、`desktop-clippy.log`。复验入口见 [开发文档](development.md#wb-204-七项只读工具验证)。本轮没有执行前端 / Python 全套、真实 AI 模型总结或安装包验收。

### 任务状态

WB-204 的七工具、领域适配、分页 / 版本 / 引用 / Unicode / 读取边界和 Windows 验证子项完成。两种 Mac 按用户要求继续待办，WB-204 总项保持未勾选；WB-119 / WB-201 / WB-202 / WB-203 的既有 Mac 待验收状态不变，主任务仍为 21 / 49。WB-205 的 Resources、后续产品开关和分发 / 真实客户端组合验收仍未交付，不扩大本轮范围。


### 独立审查

本批 Windows 限定交付为 APPROVED，Review mode 为 CHILD_AGENT，策略为 PARTITIONED_PLUS_INTEGRATION。完整覆盖 13 个代码 / 可执行文件、8 份文档增量及跨层关系，Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2。已核对分页、Unicode 标量、不可变文档版本、笔记修订、引用范围、公开 DTO、错误恢复及实际 SDK / IPC / 进程证据；1 MiB 明确仅为内部 IPC 响应上限，不误称 SDK 外部总封包上限。

代码身份为 `3f6bfdfd4e7c6170b4381865cfd74ba5fe48d4060bb2bddc3fbef23ebdbe5f51`，清单见 `.local/wb204/code-final.json`；完整交付清单见 `.local/wb204/final-target.json`。审查者独立复算全部哈希并核对基线后变更集合一致。审查后仅回填本节元信息并复核，代码和验收结论不变；macOS 两种架构仍为 UNAVAILABLE，Resources / UI / 安装包和真实 AI 总结保持后续范围。


## 2026-10-28：受限 Resources 与统一读取边界

对应 WB-205 的实现及 Windows 部分。起始仍为 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，以 `.local/workbench-batch15-baseline/` 的 417 文件清单及源码快照保存此前未提交成果。本轮未提交、推送、安装工具、修改日常客户端配置或使用真实资料 / 凭据。

### 交付

新增 SDK Resources 处理：资产、文档、笔记三个静态 URI 模板和 application/json 文本读取。resources/list 返回空目录，不枚举全库；模板 / 目录没有下一页，外来 cursor 明确失败，不开启订阅或变更通知。所有资源直接复用原 Adapter 类型化读取、现有 IPC 方法及同一 Core 公开 DTO，没有第二套读库、文件或网络通道。

URI 使用固定 yingqu://assets 命名空间，校验规范路径、UTF-8 百分编码、路由、查询字段白名单、重复值和数值形状。资产 / 文档 / 笔记归属、分页、版本 / 修订、Unicode 范围、引用和内部 IPC 大小限制均由原工具服务执行；资源缺失按 MCP 规范使用 -32002 和原领域消息，Tools 保持原错误码，数据访问范围不变。

URI 编解码复用 url 和 percent-encoding 2.3.2。Cargo 只将已锁定的 percent-encoding 增加为可选 mcp feature 的直接依赖边，锁文件没有新增 / 删除包或版本变化；没有更改数据库结构、UI、打包或发布配置。公共 URI、结果、错误及缓存边界见 [Resources 契约](mcp-resources.md)。

### 验证

| 验证 | 结果 |
| --- | --- |
| URI / 模板单元测试 | PASSED；2 项覆盖三个模板、默认参数、Unicode / 特殊 ID、查询解析、路径规范化 / 穿越形式、畸形编码、私有命名空间、重复字段、非法数值及覆盖 asset_id 的拒绝 |
| 资源 / 工具成功读取 | PASSED；官方 TS SDK 经真实 stdio / Windows 管道完成 12 对结果深比较，资产 / 文档分页、归档读取、转义 ID、Unicode 范围、最大长度和引用分页与工具一致 |
| 同域失败 / 坏 URI | PASSED；14 对工具 / 资源同域失败与 18 项非法 URI，覆盖归属、缺失、版本 / 修订冲突、分页 / 范围 / 引用上限、超大响应及任意路径 / 私有字段尝试；错误后可继续读取 |
| 隐私哨兵 | PASSED；隔离库确实含合成 Cookie / 代理密码 / UA，fixture 只回传配置存在标志；工具与资源返回及服务端线报文不含合成凭据、内部媒体路径或签名地址，未使用 OS credential store |
| 不依赖 Resources | PASSED；另一轮 --library 完成七工具读取，官方 TS 客户端线报文中没有 resources/* 请求，工具流程不依赖资源 API |
| 旧 / 新协议及回收 | PASSED；旧 initialize 与现代 discovery 均可读资源，现代保守 TTL 为 0；继续通过服务关闭、Core 退出、EOF、stdin 保持打开时自然退出和重新开启后的新进程连接 |
| Core 全量回归 | PASSED；194 通过、2 项既有 GitHub 在线测试忽略，不将在线项计为通过 |
| 无 MCP SDK feature | PASSED；6 项读取 / 边界测试不启用 mcp feature 仍通过，含合成私有配置不泄露的断言 |
| 构建 / Clippy | PASSED；--locked 构建适配器与 fixture，--features desktop,mcp --all-targets -- -D warnings 通过；初始字段赋值 lint 已修正，保留初始日志 |
| 文档 / 格式 / 差异 | PASSED；本批 Rust / JS 语法、本地链接、空白和任务状态检查通过，既有工作区变更保留 |
| macOS Intel / Apple Silicon | UNAVAILABLE；无对应机器，编译与资源 / Unix socket 原生边界继续待验收 |
| UI / 安装包 / 模型总结 | NOT_RUN；产品开关、分发和真实 AI 组合验收仍由 WB-206～WB-208 承接 |

最终资源模式证据为 `.local/wb205/run-XQi1X7/`，含 summary、Tools / Resources 双向报文、深比较结果和各生命周期分支；首轮资源运行 run-W0dh0G 同样通过。Tools-only 运行 `.local/wb204/run-YE4tJz/` 的七工具检查通过，独立报文检查保存为 `.local/wb205/boundary-proof.json`，同时核对最终资源运行的服务端响应不含哨兵值。构建与回归日志在 `.local/wb205/build.log`、`resource-unit.log`、`read-boundaries.log`、`read-without-sdk.log`、`core-regression.log`、`desktop-clippy.log`。复验命令见 [开发文档](development.md#wb-205-resources-与边界验证)。本轮没有重复前端、Python 或真实 AI / 安装包验收。

内部 IPC 响应的 1 MiB 上限继续复用，不把它描述为外部 MCP 总封包上限；资源内容只有单份 JSON 文本，工具的双表示可能放大外部封包。合成凭据只用于证明确有私有状态却未返回，不能推断系统 Keyring 或真实用户授权已接受过本轮测试。

### 任务状态

WB-205 的三类 URI、模板发现、受限资源读取、统一工具边界和 Windows 验证子项完成。macOS 两种架构按用户要求继续待办，WB-205 总项不勾选；WB-119 / WB-201～WB-204 的既有 Mac 待办不变，主任务仍为 21 / 49。设置、安装包及真实 AI 总结组合验收仍是后续任务，不将当前 SDK 资源调用等同于 P1B 整体完成。


### 独立审查

本批 Windows 限定交付为 APPROVED，Review mode 为 CHILD_AGENT，策略为 PARTITIONED_PLUS_INTEGRATION。完整覆盖 9 个代码 / 可执行 / 依赖文件、8 份文档增量及跨层集成，Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2。已复核 URI 解析、公开 DTO、同域范围及错误、隐私哨兵、Tools-only 报文、实际结果计数和平台边界；服务端响应无私有哨兵，资源模式和 Tools-only 模式的 TypeScript stderr 均为空。

代码身份为 `47d7d567718255f70f9955034a5116106349449c00e274e29de0ec03bfc91cae`，清单见 `.local/wb205/code-final.json`；完整交付清单见 `.local/wb205/final-target.json`，由审查者独立复算并确认基线后变更集合一致。审查后仅回填本节元信息并复核，不改变代码、验证结果或范围；macOS 两种架构仍为 UNAVAILABLE，真实 Keyring、AI 模型、产品 UI 和安装包继续不计为本轮通过。

## 第十六批：WB-206 MCP 设置与连接配置

对应 WB-206 的实现与 Windows 部分。起始 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，按 `.local/workbench-batch16-baseline/` 的原始文件哈希和快照保留前十五批脏改。未提交、推送、安装工具、修改日常 AI 配置或使用真实资料 / 凭据。

### 交付

系统设置增加 MCP 分类，复用 Core 已有默认关闭与启停生命周期，展示读取、开关操作、真实运行和异常状态，提供持续错误、刷新与轮询。只读配置默认 Codex TOML，可切换通用 JSON；Rust 按当前主程序同目录的实际适配程序与实际 IPC 端点生成，原生复制前复核当前状态。程序缺失或路径不可转换时不提供虚假配置，禁止开启；已运行服务仍可关闭。

界面说明知识库只读、内容可能交由客户端模型处理、重启默认关闭、退出断连与仅支持本机 stdio；未增加写入、HTTP、公网、模型请求或安装包分发。命令与界面契约见 [MCP 设置](mcp-settings.md)。

### 验证

| 验证 | 结果 |
| --- | --- |
| 新配置模块单元测试 | PASSED；2 项，覆盖缺少程序、关闭无配置、中文空格路径、command / args 独立和失败监听不输出配置 |
| `npm run build` | PASSED；类型与生产构建通过，保留已有 use client / sourcemap / chunk-size 提示 |
| `cargo check --features desktop,mcp --bin yingqu` 与 `cargo clippy --features desktop,mcp --all-targets -- -D warnings`（均 `--locked`） | PASSED；三条新增 Tauri 命令可编译并已注册 |
| Core 全量 `cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1` | PASSED；196 passed / 2 ignored，后者为已有在线 GitHub 检查；日志 `.local/wb206/core-tests.log`。首次遗漏文档要求的串行参数，179 passed / 17 个 Telegram 等待超时 / 2 ignored；按既定串行命令完成，无改动断言 |
| 浏览器 `tests/mcp-settings.browser.js` | PASSED；5 组结果，三个视口 1440×900 / 1280×800 / 1024×680，默认关闭、键盘开关与格式选择、复制、错误恢复、启停中禁用、迟到读取、返回设置、服务异常、缺程序、读取失败及横向溢出；截图 `.local/wb206/settings-*.png` 已检查 |
| Windows 原生界面与剪贴板 | PASSED；复用独立 `me.disign.yingqu.p1a-wb206-empty` 标识和 marker 空库，实际程序位于 `.local/wb206/中文 App With Spaces/映取 验证.exe`；`tests/mcp-settings-native.browser.js` 验证起始关闭、后端拒绝关闭时复制、开启、两格式原生剪贴板与预览一致、路径和端点一致；证据 `.local/wb206/native-copy.json`、`native-settings.png` |
| 复制配置的实际可用性 | PASSED；`scripts/verify-mcp-config.mjs` 在 `.local/wb206/config-5tVeMe/` 中通过 tomllib / JSON 逐项等价检查，官方 TS SDK 启动该中文空格路径的程序、握手和读取分类；Codex CLI 0.162.1 使用原样复制 TOML 发现七工具。所有客户端 home 与环境隔离，未发起模型 turn |
| 关闭与缺适配程序 | PASSED；原生关闭后显示关闭且复制禁用，旧配置实际退出 1 / IPC_UNAVAILABLE / stdout 为空，证据 `.local/wb206/config-wUakAl/`；临时移开本轮复制的 helper 后，界面禁用开启且后端返回 MCP_ADAPTER_MISSING，证据 `.local/wb206/native-missing.json`，随后已还原 |
| 设置回归与交付检查 | PASSED；既有 `tests/library-backup.browser.js` 四组回归通过；Node 语法、新文件空白、本地 Markdown 路径与本批锚点、21 / 49 主任务计数及 `git diff --check` 通过。Git 的换行提示不计为检查失败 |
| 测试进程清理 | PASSED；SDK / Codex 结束后，本轮路径下适配进程为 0，证据 `.local/wb206/cleanup.json`；原生服务从界面关闭后才终止本轮隔离测试程序，此清理不计为正常退出验收 |
| macOS Intel / Apple Silicon | UNAVAILABLE；按用户要求保留待办，Windows 不替代其设置、剪贴板、实际路径和 Unix socket 验收 |
| 正式安装包、全新设备、模型总结 | NOT_RUN；分别属于后续 WB-207 / WB-208 |

### 任务状态

WB-206 的实现、界面与 Windows 验证子项已完成；两种 macOS 继续待办，总项保持未勾选，主任务仍为 21 / 49。WB-119 / WB-201～WB-205 的既有 Mac 待办不变。未把开发隔离程序的目录测试描述为正式安装包验收。

### 独立审查

本批 Windows 限定交付为 APPROVED。Review mode 为 CHILD_AGENT，策略为 FULL_SCOPE，完整覆盖 12 个代码 / 可执行文件、11 份文档增量及跨层集成；Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2，阻塞修复轮次为 0。审查指出的两处过时总摘要已同步至 WB-206，不改动历史批次证据。

代码身份为 `949a5ab951579b83d9a29d5974bbb0cb476d1c7fa7ca092da7867bbcd72cea87`，清单见 `.local/wb206/code-final.json`；完整交付身份与逐文件哈希见 `.local/wb206/final-target.json`，由审查者独立复算。起始 420 文件清单遗漏的一份干净中文路径文档已与起始 HEAD 核对未改，记录在 `.local/wb206/exclusions.json`，不属于本批改动。终审后仅回填本节元信息并复核最终清单，不改变实现、验证结果和平台边界。

## 第十七批：WB-207 MCP 安装包与 Windows 安装验收

对应 WB-207 的实现与 Windows 部分。起始 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，按 `.local/workbench-batch17-baseline/` 的 429 文件清单和源码快照保留前十六批所有未提交成果。未提交、推送、发布、安装新工具、修改日常 AI 配置或读取真实资料 / 凭据。

### 交付

统一生产构建与 CI / Release 的 `desktop,mcp` feature，复用已有原生 bin 及辅助程序在前的声明顺序，完整许可加入 release resources。新增许可生成 / 比对、安装后原生启动检查和 Windows NSIS 安装检查器；CI Windows 实际安装 / 比对 / 卸载，macOS 配置 `.app` 内 helper / 许可与 deep / strict 签名检查。未新增 SDK 或修改 Rust 实现、锁文件版本；原有整包安装与更新机制不变。契约见 [MCP 分发](mcp-packaging.md)。

### Windows 隔离条件

以现有打包引擎和正常 release 配置生成 NSIS，额外合并 `.local/wb207/installer-config.json`：identifier 为 `me.disign.yingqu.wb207`、产品名为 `Yingqu WB207`、主程序名为 `yingqu-wb207`，均与日常应用隔离。预建实际 Roaming 下的独立空数据目录和 marker，生产 setup 不触发旧库迁移。没有更改生产配置文件中的标识、版本或用户配置。

实际安装目录为 `.local/wb207-install/wb207-final/安装 Program Files`，安装器由 NSIS 真正执行写入；没有把复制构建目录当成安装。内嵌前端为 `http://tauri.localhost/`，不依赖 Vite。此环境是当前 Windows 主机，不是全新无开发工具的机器。

### 验证

| 验证 | 结果 |
| --- | --- |
| Release 构建与 NSIS | PASSED；`npm run tauri -- build --features desktop,mcp --config src-tauri/tauri.release.conf.json --config .local/wb207/installer-config.json --bundles nsis -- --locked`；最终日志 `.local/wb207/build-isolated-installer.log`。包含前端类型 / 生产构建和三个原生 bin；保留既有 use client / chunk-size / STATIC_VCRUNTIME 提示 |
| 许可与依赖 | PASSED；`scripts/update_mcp_licenses.py --check`，当前宿主 MCP 增量 7 包加已有 percent-encoding，8 包 / 13 份完整许可；安装文本与源文件逐字节相同。Cargo 未新增包或更新版本 |
| 新打包回归 | PASSED；`.venv/Scripts/python.exe -m pytest -q tests/test_mcp_packaging.py`，7 项，覆盖缺文件、许可不符、NSIS 仅允许首个类型标记变化、helper / 其他主程序字节变化拒绝，以及 CI / Release 全新 cache 下先构建再检查许可、签名失败保留退出码 42。四个流水线用例是当前 Windows 上的 pwsh 成功 / 故障注入，不能视为 macOS 代码签名验收 |
| 静态检查 | PASSED；`cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings`、Node 语法、PowerShell 解析。没有 Rust 行为变更，不重复上一批完整 Core 回归 |
| 安装后文件完整性 | PASSED；`.local/wb207-install/wb207-final/installation.json`。安装器 SHA-256 为 `6F2FBC00EC12298C269159F76BDA617B142DB30C159B044F9E2CD699B24C83EC`，helper 为 `85EABE2EF14451D2E880DEFEFC7F793314F2B705394F07BEB835987909D6E3FC`，与构建输出全字节相同；主程序除 Tauri NSIS 首个 UNK → NSS 标记外全字节相同，许可原文匹配 |
| 实际原生 UI 与配置 | PASSED；`tests/mcp-installed.browser.js` 验证内嵌页面、首次默认关闭、启停状态及两格式原生剪贴板与预览一致，路径指向安装目录而非 target；证据 `.local/wb207/installed-copy.json` 和 `installed-settings.png`，截图已查看 |
| 系统 PATH 下实际连接 | PASSED；`verify-mcp-config.mjs ... --output .local/wb207 --minimal-path`，仅系统 PATH、独立 home、临时 cwd、无凭据。`.local/wb207/config-FIqHjS/summary.json` 记录 tomllib / JSON 等价、官方 TS SDK 启动已安装 helper / 七工具 / 分类读取、Codex CLI 0.162.1 用复制 TOML 发现七工具；没有模型 turn |
| 关闭、重启和重连 | PASSED；原生关闭后复制禁用，旧配置启动退出 1 / IPC_UNAVAILABLE / stdout 为空，证据 `config-a6RMFE/`；重新打开安装程序默认关闭，证据 `restart-copy.json`；开启后旧配置重新启动并读取通过，证据 `config-UwiEBW/` |
| 原下载引擎与原生依赖 | PASSED；安装目录的 `check_packaged_engine.py` 离线启动返回 video-engine 0.15.0、yt-dlp 2026.08.19。helper PE 导入仅系统 / UCRT DLL，无 Node / Python / Rust 开发运行时 DLL；这不替代全新设备安装验收 |
| 卸载与 CI 默认路径 | PASSED；SDK / Codex 退出后 helper 进程为 0，界面停服后才终止隔离主程序；实际卸载后主程序和 helper 均消失，证据 `.local/wb207/cleanup.json`。再以脚本默认非 Keep 模式执行完整安装 / 校验 / 卸载通过，证据 `.local/wb207/ci-install-check.log`、`.local/wb207-install/wb207-ci-path/installation.json`；主程序的测试清理不计为正常退出验收 |
| macOS Intel / Apple Silicon | UNAVAILABLE；按用户要求继续保留待办，不用 Windows 或 pwsh 模拟代替 app / DMG、签名、Unix socket 和实际客户端验证 |
| 远程 CI、正式发布、全新设备、跨版本在线更新、模型总结 | NOT_RUN；未推送或发布本轮代码，不将构建脚本接入视为远程 CI 已通过；模型组合验收属于 WB-208 |

实现中发现并修正验证问题：安装脚本不再依赖可能缺失的 OS 环境变量，而读取实际平台；主程序首次直接 SHA 对比失败后，核对固定 Tauri 源码确认打包标记与构建输出恢复行为，改为严格允许唯一官方标记变化，其他差异仍拒绝，非跳过校验。CI / Release 的离线许可检查改在完整 feature 构建后、安装检查与上传前执行，避免全新 runner 的可选桌面依赖尚未进入 cache；成功与失败两种分支均有本地回归。未将失败运行计为最终安装通过。

### 任务状态与审查

WB-207 的构建、许可、安装检查与 Windows 子项完成；两种 macOS 继续待办，总项保持未勾选，主任务仍为 21 / 49。此前各项 Mac 待验收不变，WB-208 继续负责真实 AI 组合验收。

安装主程序与 helper 已卸载，独立测试数据目录 `me.disign.yingqu.wb207` 仍保留。对该目录的递归清理被自动审批以 `blocked by policy` 拒绝，未以其他方式重试删除；marker 与空测试库已快照到 `.local/wb207/profile-snapshot/`，状态见 `.local/wb207/ci-cleanup.json`。此项是测试数据清理限制，不影响已完成的安装、连接和卸载验证。

本批 Windows 限定交付为 APPROVED。Review mode 为 CHILD_AGENT，策略为 PARTITIONED_PLUS_INTEGRATION：完整覆盖 11 个代码 / 配置 / 许可文件、11 份文档增量和跨层集成，22 文件哈希独立复算匹配，429 文件基线中的无关变化为 0。Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2，阻塞修复轮次为 1。

发现的 P1——macOS CI 中签名错误可能被随后 Python 成功覆盖——已在 codesign 后即时检查退出码，并通过两个故障注入回归验证。后续 cache 检查顺序、成功 / 失败共四个流水线用例、严格 NSIS 比对、原生安装证据和保留测试目录的限制均已复核。

代码身份为 `20c441d9f8d841572363ba0d3507214cb7cbaa7ca0fef4732b016a84895a7a3e`，清单见 `.local/wb207/code-final.json`；完整交付身份与逐文件哈希见 `.local/wb207/final-target.json`。终审后仅回填本节元信息并复核最终清单，不改变实现、验证结果或平台边界。


## 第十八批：WB-208 真实 MCP 客户端与 P1B 验收

对应 WB-208 的 Windows 部分。起始 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，按 `.local/workbench-batch18-baseline/` 的 436 文件清单和原始快照保留前十七批未提交成果。用户明确授权本机已登录 Codex CLI 对本轮合成文案做模型对比；不使用日常知识库、不复制或输出凭据、不改写日常 AI 配置。未提交、推送、发布或安装新工具。

### 交付与范围

新增带标记 / 空库门槛的合成资料构造器、真实 Codex app-server 驱动器、结果断言模块和回归测试，共 5 个可执行文件；复用现有 SDK、原生程序、NSIS 和安装检查器，没有修改产品 Rust / React / 协议 / 依赖。独立产品 `Yingqu WB208` 安装在 `.local/wb207-install/wb208/安装 Program Files`，Roaming 标识 `me.disign.yingqu.wb208`。

客户端 **Codex CLI 0.162.1 / gpt-6.1-sol** 的完整运行位于 `.local/wb208/client-ruGfcM/`。模型会话与手动协议会话分开，模型自行使用七工具筛选、分页、读取、对比，包含 4 处可核验引用。每个 ephemeral 会话只允许 yingqu 的只读工具，显式禁用 7 个继承 MCP；不假设配置覆盖会删除继承项，不保存完整有效配置或认证 stderr。使用登录存储仅发生在明确的 `--allow-account` 模式，外部模型推理与本机转录区分。

具体样本、隔离、错误和复验步骤见 [真实客户端验收](mcp-client-acceptance.md)及[开发文档](development.md#wb-208-真实客户端与模型验收)。

### 分项结果

| 检查 | 状态与证据 |
| --- | --- |
| Node / Python 验收器回归 | PASSED；`node --test tests/mcp-client-checks.test.mjs` 15 项，`.venv/Scripts/python.exe -m pytest -q tests/test_mcp_acceptance.py` 2 项；新增失败响应不能充当引文、版本 / 返回范围、动作白名单和断连语义回归，未弱化产品断言 |
| Core 全量回归 | PASSED；`cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1`，196 passed / 2 ignored，后者为已有在线检查；`.local/wb208/core-tests.log` |
| 构建与 Windows 实际安装 | PASSED；`npm run tauri -- build --features desktop,mcp --config src-tauri/tauri.release.conf.json --config .local/wb208/installer-config.json --bundles nsis -- --locked`，包含类型 / 前端生产构建；安装校验含 helper、引擎、许可和中文空格目录。见 `build.log`、`install.log` 及 `.local/wb207-install/wb208/installation.json` |
| 真实客户端协议 | PASSED；发现及调用七工具，父分类后代 + 标签 + 抖音 + 未归档联合筛选，limit=1 分页只返回 A / B，Unicode 切片及五类错误边界通过；最终 `summary.json`、`operator-tools.json` |
| 实际模型和引用结构 | PASSED；读取当前 A v2 / B v1 和各 r1 笔记，4 处原句与成功工具返回的稳定版本及 Unicode 标量范围完全一致；`model-tools.json`、`comparison.json`。结构审计保留 `semanticQuality=REQUIRES_REVIEW`，不冒充内容质量判断 |
| 模型内容质量 | PASSED；单独逐项审阅开场、画面描述、行动引导、共同策略、四处差异和三项来源复用建议，均有正确来源；明确未看视频帧、无时间戳 / 效果数据，不能推断效果优劣。见 `.local/wb208/quality-review.json`；独立审查另见下节 |
| 原生来源定位 | PASSED；实际资产详情核对 A v2 / B v1 全文、两条 r1 笔记，引用原文分别打开正确已保存版本；`.local/wb208/ui-reference-proof.json` 和六份 source / note / pinned-citation UI 文本 |
| 只读和返回范围 | PASSED；完整协议与模型阶段前后 23 张逻辑表行数 / 内容哈希相同，公开结果不含私有字段、路径 / URL 哨兵；`readonly-proof.json`、`database-final-before.json`、`database-final-after-model.json`、`ui-calls-during-model.json` |
| 服务关闭及重新开启 | PASSED；旧调用 `-32603 / Transport closed`，新会话 MCP 初始化失败；开启后新会话固定文档 A v2 读取成功；最终 `summary.json` |
| 应用故障断连与重启 | PASSED；只终止核实路径的测试主程序，旧 transport / 新会话均失败；普通重启 UI 默认关闭，客户端初始化失败；原生设置开启后新会话重连成功；`restart-off-ui.txt`、`restart-on-ui.txt`、最终 `summary.json` |
| 客户端和安装清理 | PASSED；客户端 stdin 关闭后自然退出 0、forced=false，本轮 helper 进程为 0；原生设置关闭 MCP 后才终止隔离主程序，卸载 exit0 且主程序 / helper 均移除。`cleanup.json`；此清理不计正常托盘退出验收，带标记合成资料目录保留，没有递归清理 AppData |
| 交付检查 | PASSED；Node 语法、新文件空白、本地 Markdown 路径 / 本批锚点、21 / 49 主任务计数及 `git diff --check`；清单与身份见 `.local/wb208/final-target.json` |
| macOS Intel / Apple Silicon | UNAVAILABLE；继续待各自安装后客户端、模型组合、原生来源、Unix socket 和生命周期验收 |
| 真正视频画面、人工视听、全新设备、正式发布 | NOT_RUN；占位视频与合成制作笔记不支持多模态或真实视频效果结论，未自动扩展到 P2 |

首次非 PTY 模型运行在生命周期阶段因 stdin 结束而中断，记录为 `ABORTED_HARNESS`；两次继承配置试运行均在模型前失败，不能算通过。最终完整运行独立留存。首次 UI 访问产生的访问记录保留，最终协议 / 模型快照期间隐藏测试窗口并记录实际 Tauri 命令无 visit；后续 UI 来源导航在只读快照之后，不把其合法访问记录误判为 MCP 写入。

一次带远程调试参数的重启被自动审批拒绝，未重试绕过；普通启动与原生 Computer Use 完成重启验收。既有安装 UI 检查器的硬编码截图路径使 `.local/wb207/installed-settings.png` 被当前截图覆盖，已另存 `.local/wb208/setup-settings.png` 并记录来源；旧路径图片不再作为 WB-207 原图，其余旧证据和产品文件保留。

### 任务状态

WB-208 的协议、真实客户端 / 模型、来源 / 只读、生命周期及 Windows 子项已完成。WB-119 / WB-201～WB-208 均因两种 macOS 待办保留总项未勾选，主任务仍 **21 / 49**。P1B 为 **Windows 限定验收通过**，不是全平台完成。

### 独立审查

本批 Windows 限定交付为 APPROVED。Review mode 为 CHILD_AGENT，Tier 3，策略为 PARTITIONED_PLUS_INTEGRATION。完整覆盖 5 个代码 / 可执行文件、13 份文档增量与集成，18 文件哈希独立复算匹配，436 文件基线无范围外变化，31 份证据哈希匹配。Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2，阻塞修复轮次为 1。

审查修复了失败 / 缺失工具响应仍可能通过引文审计的 P1，并核对动作白名单、断连错误语义、客户端配置隔离及样本描述。独立语义判断 PASSED，4 处引用与真实读取 / 原生界面一致，23 表只读快照完全相同；故障退出、测试清理和截图覆盖边界如实记录。

代码身份为 `001d7517844b26fe17cd7e561a39afb402415431ad428a7cd09d658010e7781c`，5 文件清单见 `.local/wb208/code-final.json`；完整交付清单、最终差异与证据清单分别位于 `.local/wb208/final-target.json`、`final.patch` 和 `evidence-inventory.json`。终审后仅回填本节和质量记录中的审查元信息并复核最终身份，不改代码、实际结果、平台边界或既有成果。


## 第十九批：WB-301 文案时间来源与精度验证

起始 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，保留 `.local/workbench-batch19-baseline/` 442 文件基线中的前十八批成果。本批只新增 2 个实验脚本、1 份回归测试及必要文档；不改生产 Rust / Python、数据库、协议、运行依赖或 UI，没有提交、推送、发布、安装工具 / 模型、使用凭据或调用云端模型。

### 结论与后续范围

现有 CPU 固定 CLI `--srt` 输出的是 VAD 音频窗口；CUDA 已暂存对应毫秒区间、实际裁剪 WAV 和逐段正文。当前生产只合并全文，段落时间未返回或落库。进度计数不是时间，10ms 帧网格不是 ±10ms 精度。

本轮明确选择 **FSMN-VAD 语音段级范围**，复用已有原生程序和模型。最长窗口实测 30 秒，可能多句共窗或在词中切分，不承诺句子 / 逐字 / 逐帧同步。额外对齐仅作成熟方案调查，不引入 WhisperX / faster-whisper 或自研对齐算法。

音频晚于视频开始 1500ms 的控制样本证明：当前提取会丢失延迟，不能直接把 WAV 时间用作视频播放时间。复用 FFmpeg 的候选归零滤镜在该样本精确保留 24000 个静音采样帧；这不是所有容器 / 原点已通过，WB-302 须先验证媒体零点，再实现段落 artifact、Rust 校验与不可变文档 / 文件绑定，旧文案不回填时间，整理稿不自动继承。

来源、精度、质量观察和 WB-302 具体边界见 [时间来源与精度验证](transcript-timing-validation.md)。本轮没有执行 WB-302 的生产落库，也没有将更细时间能力视为已完成。

### 实际验证

| 检查 | 状态 / 证据 |
| --- | --- |
| 新证据解析和既有转录回归 | PASSED；`.venv/Scripts/python.exe -m pytest -q tests/test_transcript_timing.py tests/test_transcription.py`，34 项，含 18 项新时间解析 / 非法范围 / 原始窗口映射 / 静音摆放 / 输入不变回归；`.local/wb301/python-tests.log` |
| Windows 短中文 | PASSED；3000ms，独立 VAD、CPU SRT、生产 CUDA 均 1 段，区间 `[420,2980)`；当前生产 CPU / CUDA 完成结果均没有 segments |
| Windows 停顿与中英混合 | PASSED（时间映射）；18476ms，三条 VAD / CPU / CUDA 范围完全对应，已知 1800ms 静音与素材摆放可核查；尾段包含 884ms 的追加静音，存在中文错字，不计全文质量通过 |
| Windows 连续长语音 | PASSED（时间映射）；66000ms，4 组范围，对应采样 / 顺序 / 正文合并通过，最长 30000ms；首 30 秒 CPU / CUDA 输出 99 / 9 字符，存在重复语音压缩 / 漏识别，不能用完成片段数证明文字完整 |
| Windows 自然长口播 | PASSED（时间映射）；297586ms（毫秒显示向下取整），79 组范围，CPU SRT 与独立 VAD 完全一致，CUDA 79 份裁剪采样 / 正文映射通过；最长 30000ms、最短 970ms，未作完整人工词边界标注或 WER / CER |
| 主实验总结果 | PASSED；`.local/wb301/run-complete/summary.json`，共 87 组窗口，逐样本 result / SRT / VAD / 原生日志 / CUDA 暂存保留；命令见 [开发文档](development.md#wb-301-文案时间来源验证) |
| 固定来源与输入完整性 | PASSED；固定上游 CLI 通过 HTTP 读取确认 `--srt`，源码记录 `.local/wb301/research/source-check.json`；8 份程序 / 模型哈希及工具版本见 `environment.json`，三个原始音频输入前后哈希一致 |
| 媒体时钟实验 | PASSED（两种控制样本）；`scripts/verify_transcript_clock.py --output .local/wb301/clock`，0 / +1500ms Matroska PCM16 音轨经 FFprobe 确认，当前输出丢延迟，候选滤镜精确保留原 PCM 与应有静音；`clock/summary.json`。FFprobe 仅作测试工具，不加产品依赖 |
| 资源回收 | PASSED；实验退出 0，本轮路径参数下 CPU / VAD / CUDA 子进程为 0，`cleanup.json`；样本和证据保留，没有安装应用或改写用户资料 / 后端选择 |
| 交付检查 | PASSED；Python AST、新增文件空白、本地 Markdown 链接 / 本批锚点、主任务 21 / 49 和 `git diff --check` |
| macOS Intel / Apple Silicon | UNAVAILABLE；各自 CPU CLI / VAD / 媒体时钟与原生定位条件继续待验收，不要求 Windows CUDA |
| UI、原生播放器精确定位、生产段落 / 迁移 / 打包 | NOT_RUN；本轮没有改动这些实现，分别由后续任务和各平台实测承接 |
| 词 / 句对齐、完整字幕质量、全文人工准确率 | NOT_RUN / 不承诺；缺少真值集和对应时间来源，实验结构成功不替代这些结果 |

首轮 `.local/wb301/run-final/` 因实验驱动器未传 CUDA 设备标识，启动时环境字符串校验失败；修正为调用生产 `require_device()` 后，在全新 `run-complete` 目录完整重跑，原失败记录保留，不把它算作产品回归或通过证据。官方英文样本仅下载到本轮测试目录，没有下载新的运行依赖。自然中文使用已有隔离 ASR 缓存，本轮未读取日常库或原视频目录。

### 任务状态

WB-301 的来源调查、Windows 实跑和实施方向子项完成。macOS Intel / Apple Silicon 继续待办，WB-301 总项保持未勾选，主任务仍 **21 / 49**；WB-119 / WB-201～WB-208 的既有 Mac 待办不变。精确同步继续阻塞，全文能力保持现状。

### 独立审查

本批 Windows 限定交付为 APPROVED。Review mode 为 CHILD_AGENT，Tier 2，策略为 FULL_SCOPE。完整覆盖 3 个 Python 文件、8 份文档增量与相关生产调用契约，11 文件和 38 份证据哈希独立匹配，442 文件基线无范围外变化。Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2，阻塞修复轮次为 0。

审查者独立核对全部 87 份 CUDA 裁剪 PCM 与源音频对应采样逐字节一致、输入哈希不变；时钟样本丢失延迟与候选补齐静音的结论成立。固定 CLI 源码支持 VAD 窗口时间而非文本对齐的结论。正文错字、长段漏识别和无人工真值已准确单列，不把结构映射成功当作文本质量或精确同步通过。

代码身份为 `1ae6b3eb74f37b2da6c0a4d3e5a520b9ddc47f21bdcd27ff6553d7551fb7a1c8`，清单与差异见 `.local/wb301/code-final.json`、`final-target.json`、`final.patch`、`evidence-inventory.json`。终审后仅回填本节元信息并复核最终身份，不改变实现、验证结果、平台边界或前序成果。


## 第二十批：WB-302 媒体时间原点与可追溯段落存储

起始 `master` / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，保留 `.local/workbench-batch20-baseline/` 的 446 文件基线。用户授权范围为统一媒体零点、可追溯语音段落库、版本 / 文件绑定和失败恢复；没有启动 WB-303 UI，未提交、推送、发布、安装工具 / 模型或使用凭据。

### 交付与契约

生产提取保留呈现 PTS，使用 `aresample=async=1:first_pts=0` 补正延迟 / 裁负时间，不减掉首包或容器起点。CPU 使用已有 `--srt`；CUDA 保存真实 VAD 区间 / 裁剪正文映射，私有 schema v1 文件仅以固定描述符通过 JSONL 交给 Rust。engine 0.16.0 与 Rust 精确握手，协议仍 v3；权重 / 已安装模型就绪 manifest 不变，新结果 pipeline 标记 media-zero-v2。

Rust 校验文件、大小、结构、元数据、采样边界、顺序、重叠和全文一致性后，赋真实操作 ID；schema v4 的时间头 / 有序段落随不可变识别文档、文件 ID / Source 指纹、模型 / 后端 / pipeline 同事务保存。旧文案无时间，整理稿不继承；同秒不同操作仍生成不同版本，重复同一结果仍幂等。

schema 1 / 2 / 3 先做 WAL 一致性备份再事务升到 4。逻辑备份 writer v2 / reader v1、v2 保留时间、关系和历史版本，恢复重定位后重算识别键，避免旧结果重复生成版本。详细边界见 [段落契约](transcript-segments.md)，复验见 [开发文档](development.md#wb-302-段落存储与媒体时钟验证)。

### 验证结果

| 检查 | 状态 / 证据 |
| --- | --- |
| Python 相关回归 | PASSED；54 项（`python-target-final.log`），包含原转录 / CUDA、SRT 解析、错误区间、EOF、固定时间文件和 schema 已知版本 fixture |
| Python 全量 | PASSED；499 项，`python-full-final.log`；先执行精确版本的 `m1-probe` 构建，未访问真实平台 |
| Core 全量 | PASSED；`core-tests-final.log`，202 passed / 2 ignored，后者为已有在线检查；随后只新增 v1 兼容测试并收紧同秒操作测试，定向 timing 7 项通过（`rust-timing-final.log`），没有修改生产行为 |
| Clippy / Rustdoc | PASSED；`--features desktop,mcp --all-targets -- -D warnings`，以及 `--features mcp --no-deps`；`clippy-final.log`、`rustdoc.log` |
| 前端类型 / 构建 | PASSED；`npm run build`，`frontend-build-final.log`；只扩展 API 类型，保留既有 sourcemap / use client / chunk-size 提示 |
| 时间原点 | PASSED；`clock-pts-zero/summary.json` 11 组，6 组 PCM 逐字节一致，AAC / Opus 有损起音观测为 0～16 个采样帧（最多 1ms），在 codec frame 容忍内；不等同词精度 |
| Edge 解码 / seek | PASSED（限定坐标）；`browser-clock-pts-result.json` 5 组，含非零 MP4 的 5.2 / 6.6 / 7 秒；脚本为 `browser-clock-pts.js`，使用 Vite byte-range 服务，不能替代 Tauri 或浏览器音画同步实测 |
| 真实 Core + 源码 CPU / CUDA | PASSED；`core-cpu-final/report.json`、`core-cuda-final/report.json`；真实导入、转换、推理、时间存储、两次识别 / 原版本不变、重开与 Source 绑定，输入 hash 不变 |
| 打包 sidecar | PASSED；最终 PyInstaller 重建和 `check_packaged_engine.py` 确认为 0.16.0，真实 Core 驱动 packaged CPU 通过相同流程；`pyinstaller-final.log`、`packaged-check-final.log`、`core-packaged-final/report.json` |
| 文件与版本 / 恢复 | PASSED；7 个 timing 测试包含历史版本、Source 变化 / 重新关联、整理稿无时间、写入中途失败回滚及重试、v3 备份 / 升级及失败恢复、v1 / v2 含 / 不含媒体恢复和恢复后幂等；核心既有取消 / 无语音 / 写状态失败 / 重开测试继续通过，新增 6 类非法产物也保留旧时间 |
| 清理 | PASSED；`cleanup.json`，真实 Core shutdown 后本轮 native workers 为 0，独立 Edge / HTTP / Vite 服务已结束；保留带标记测试资料及只读模型硬链接，不改日常后端选择 |
| 交付检查 | PASSED；新文件空白、Python AST、3 个新增 Rust 文件 rustfmt、本地 Markdown 链接 / 本批锚点、21 / 49 主任务计数及 `git diff --check` |
| macOS Intel / Apple Silicon | UNAVAILABLE；CPU / VAD、PTS、文件 / 版本、恢复和打包各自继续待验收 |
| 完整 Tauri GUI、正式安装包 / 发布、精确音画同步 | NOT_RUN；本轮实际 native Core / 打包侧进程不替代这些验收，未增加同步 UI 或逐字对齐 |

### 试跑与实际限制

首轮 Core 全量为 201 passed / 1 failed / 2 ignored，失败为 workbench 迁移测试仍预期 schema 3，按新的 schema 4 契约修订后全量通过。首轮 Python 为 488 passed / 11 failed，原因是漏重建 `m1-probe`，旧握手拒绝 0.16.0；按开发顺序重建后 499 项通过，未弱化协议检查。Clippy 发现新增枚举范围写法，应改 `Some(1..=3)`，修正后通过。

最初按 WB-301 候选减掉容器起点，Edge 实测非零 MP4 不能 seek 到 0.2 秒、会落在 5 秒，故改为保留 PTS 零点；11 组矩阵和最终 Core 重跑验证的是修正后的生产代码。旧试跑 / 旧构建保留，不冒充最终通过。首次浏览器实验还使用了不支持 byte ranges 的简单 HTTP 服务，并有帧回调等待被后续导航中断；最终改用 Vite 和有界 seek 等待，错误记录保留。

异常 MP4 的 Edge duration 为 8.804 秒，与原始音轨末端 PTS 不完全相同。本轮只验证列出的位置，不声称整个文件在内置播放器均可播放；后续 WB-303 须使用实际 seekable / duration 并处理不支持的范围，不能用提取 WAV 长度直接替代播放器长度。语音段仍可达约 30 秒，不承诺句子 / 词 / 字同步或识别全文正确率。

### 任务状态

WB-302 的实现和 Windows 子项完成。macOS 两种架构继续待办，总项保留未勾选，主任务仍 **21 / 49**；WB-119 / WB-201～WB-208 / WB-301 的既有 Mac 待办不变。没有自动推进片段、同步界面或更细对齐。

### 独立审查

本批 Windows 限定交付为 APPROVED。Review mode 为 CHILD_AGENT，Tier 3，策略为 PARTITIONED_PLUS_INTEGRATION。完整覆盖 Python / 时钟 9 文件、Rust / DB / TS 17 文件、浏览器验证脚本 1 文件和 11 份文档；38 文件、两个 checkpoint 和 29 份证据哈希独立匹配，446 文件基线无范围外变化。Review gate 为 PASSED，Coverage 为 COMPLETE，Freshness 为 FRESH，无剩余 P0 / P1 / P2，阻塞修复轮次为 0。

已复核固定产物的跨语言字段、PTS 零点、真实采样、不可变版本 / Source / 操作事务、旧结果保留、备份兼容和恢复幂等；CPU、CUDA、打包 CPU 的真实两版本和重开记录吻合。WebM 起音误差和 Edge 的限定 seek / 异常 MP4 范围如实记录，没有宣称精确同步或全平台通过；审查指出的一处当前总摘要旧状态已修正，历史证据保留。

代码身份为 `35892b8c16ccf19e871c631b88148a3c39c93a980559b07957be5aad8063d2f5`，完整身份、差异和证据清单见 `.local/wb302/code-final.json`、`final-target.json`、`final.patch`、`evidence-inventory.json`。终审后仅回填本节审查元信息并核验最终身份，不改动代码、实际结果、平台边界或原有成果。


## 第二十一批：WB-303 片段收藏与范围播放

### 本批范围

在 WB-302 媒体 PTS 零点和文案时间存储上，增加片段收藏、范围播放、固定文案 / 文件引用及备份恢复配套。起始仍为 master / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，先保存 453 文件基线 `.local/workbench-batch21-baseline/`，只审本轮增量。未提交、推送、发布、安装工具、访问日常知识库或使用凭据；仅复用带标记的合成验收 profile。未实现 WB-304 关键帧、WB-305 同步阅读或扩大 MCP 能力。

收藏只存引用，不裁剪视频；手动标记或输入秒数，填写标题 / 理由，亦可从已保存文字选区带入固定版本原句。片段列表可播放、查看原版本和删除；重新关联不重写历史源，缺失 / 变化只阻止播放，引用仍可读。两种单篇导出保护集合已纳入片段历史源。完整字段、错误和 UI 约定见[片段契约](asset-clips.md)。

schema v5 迁移前一致性备份并事务升级。逻辑备份 writer v3、reader v1 / v2 / v3；恢复校验资产 / 文件 / 文案版本 / 原句及来源链指纹，不恢复原绝对路径。相同历史 Source 共用安全占位身份，支持恢复后再次备份，不把旧文案混配到新源。

### 实际验证

证据根目录 `.local/wb303/`；Playwright 截图位于 `output/playwright/wb303-*.png`。

| 检查 | 状态及证据 |
| --- | --- |
| Core 全量 | PASSED；`cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1`，209 passed / 2 ignored（已有在线检查），`core-tests.log`。之后仅受影响库逻辑定向复验及新增 Core 用例，不重复全套 |
| 最终库 / 恢复 | PASSED；`library-final.log` 34 项，含 6 项新片段测试：无时间手动收藏、Unicode / 版本 / 来源链、错资产 / 文件 / 原句、缺失 / 变化 / relink、范围、迁移失败重试；含 / 不含媒体与 relink 四组合均完成第二次备份恢复；坏来源混配、空 ID、范围和旧版本混片段拒绝 |
| Core 配套 | PASSED；`core-clips-final.log`，成功事件、失败不发事件、重开保留、变更拒播及两种导出拒绝覆盖历史视频；删除仅收藏，文件字节保持 |
| Clippy / Rustdoc | PASSED；desktop,mcp all-targets `-D warnings`，`clippy-final.log`；mcp `--no-deps`，`rustdoc.log`（后续 Source / 导出查询修复未改变 Rustdoc API） |
| 前端类型 / 构建 | PASSED；`npm run build`，`frontend-build-final.log`；保留既有 use client、sourcemap 和 chunk-size 提示 |
| 片段浏览器 | PASSED；`browser-clips-r5.log` 9 组，三视口 1440×900 / 1280×800 / 1024×680，真实解码 / 到点暂停、固定 v1、草稿保护 / 保存失败、错源缺源、特殊 MP4 及不截短；额外 DOM 事件故障注入验证播放中范围收缩 / 提前 ended，双草稿一起保存后离开。故障注入不冒充真实文件收缩实测 |
| 既有 UI 回归 | PASSED；`browser-video.log` 7 组、`browser-writing.log` 5 组、`browser-writing-failures.log` 9 组，保留解码失败、无画面、迟到结果、切文件 / 离开回收、长正文、固定引用、保存冲突与重试 |
| Windows 原生 | PASSED；隔离 identifier `me.disign.yingqu.p1a-wb303-final-upgrade`，重建 `p1a-acceptance`，真实生产 UI / 四命令 / SQLite / WebView2 asset protocol。`native-clips-final.log` 验证保存、引用、错文件拒绝与 MP4；最终播放器 `native-player-final.log` 观察普通片段在 1.2s、特殊 MP4 在 6.6s 暂停，20s 终点完整拒播 |
| 原生重开 / 源状态 | PASSED；空闲测试实例结束后用最终 backend 重开，三条 ID / 范围 / 来源和 v1 文案保持（`native-restart.log`）；只改验收副本后分别返回 `CLIP_SOURCE_CHANGED` / `FILE_MISSING`，旧原文仍可读（`native-source-states.log`）。验后恢复副本的字节 / mtime，最终实播再次通过；不把空闲强制重开宣称为优雅关闭测试 |
| Python 验收 fixture | PASSED；既有隔离 seeder 兼容检查，`python-fixture.log`；生产 Python / ASR 未改，本轮不重跑模型 |
| macOS Intel / Apple Silicon | UNAVAILABLE；原生解码、范围、文件 / 文案、迁移与恢复分别保留待办 |
| 正式安装包、发布、精确音画 / 逐帧精度 | NOT_RUN；本轮原生是开发构建，不替代安装包及跨平台验收，不承诺精确剪辑 |

### 特殊 MP4 与试跑

正 PTS MP4 的 seekable 下界报 0，真实 seek 0 落 5 秒；Chromium duration 可随定位 / 解码由 6.692 / 8.804 秒变化至 10.2 / 10.9 秒。当前实现取实际 seek 下界及动态可播放范围，保存和每次播放都完整包含校验，起点落点不符则拒绝。界面不固定上轮观测的 8.804s，不用转录音频末端代替播放器时长。范围是当前渲染器能力，不是所有渲染器的共同媒体长度；不足时明确提示，不改写收藏。范围试听受帧率和调度影响，不宣称逐帧截断。

首轮浏览器选区模拟漏触发 selectionchange，后已按既有用例修正；开发 HMR 曾触发未保存草稿的 beforeunload，保留记录并在冻结代码后复验。最初断言把 8.804s 写死，实测动态范围后改为校验 5s 下界及实际范围；初版客户端 Error 被通用错误映射吞掉已修正。新增范围变化处理的试跑还覆盖了起播前错误信息的覆盖问题，最终只在播放中复验收缩，暂停后恢复仍从 onPlay 重新校验。

原生准备最初替换第二个测试文件却保留旧登记大小，正确返回 SOURCE_CHANGED；仅对带标记的合成旧任务 / 文件同步登记值后重验。另一次自动化把跨 CDP 边界异常当原始字符串比较、一次 UUID selector 未加引号，均为验证脚本错误；最终在浏览器内捕获原始命令错误并使用带引号定位，未放宽产品校验。失败日志保留，不计通过。初次组合命令被自动审核拒绝后拆为明确的补丁与服务启动完成，不存在未完成的批准动作。

### 任务状态与交付复审

WB-303 的实现与 Windows 子项已完成；两种 macOS 继续待办，总项未勾选，主任务仍 **21 / 49**。既有 WB-119 / WB-201～WB-208 / WB-301 / WB-302 的 Mac 待办不变。

本批采用 CHILD_AGENT / Tier3 / PARTITIONED_PLUS_INTEGRATION。后端 12 文件、前端 14 文件（共享一项 fixture）的分区审查均 PASSED；修复三轮具体阻塞：恢复源混配、历史源二次备份身份、播放中范围收缩 / 提前 ended。完整覆盖与最终身份由下述终审记录确认。


终审结果为 Windows 限定 **APPROVED**：Review gate PASSED、Coverage COMPLETE、Freshness FRESH，无剩余 P0 / P1 / P2。独立完成后端 12 文件、前端 14 文件（共享一项 fixture）、10 份文档及跨层集成审查，28 项证据哈希匹配。未修改既有成果，主任务 21 / 49 与 Mac 待办保持一致。

最终代码身份 `1a09bd54ba2ae4a8d20d02c1ea347b9a4cb7ad937a7589fe84747b6359126fc6`；内容、增量与证据清单见 `.local/wb303/code-final.json`、`final-target.json`、`final.patch`、`inventory.json`、`evidence-inventory.json`。终审后只回填本段审查元信息并核对新文档身份，代码、验证结果、范围和平台边界未变。

## 第二十二批：WB-304 关键帧提取与资源管理

### 范围与实现

起始 master / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，保存 `.local/workbench-batch22-baseline/` 的 461 文件基线，仅审本轮增量。实现当前位置取帧、片段均匀采样、图片资源 / 来源状态、预览删除与备份恢复；未实施 WB-305、WB-312，没有提交、推送、发布、安装工具 / 模型、使用凭据或访问日常知识库。

复用 FFmpeg / Pillow 与现有 Core worker、PythonBridge；engine 0.17.0、协议 v3。保留原始呈现 PTS，取请求位置后的第一帧，记录请求与实际时间，完整保留黑帧并修正 SAR 显示比例。schema v6 保存 PNG BLOB 和不可变来源，writer v4 / reader v1～v4 随无媒体备份保留图片；PNG 使用已锁定的 image-png 完整解码校验，许可纳入 release 资源。详见[关键帧契约](asset-keyframes.md)。

失败或取消在实际引擎 / FFmpeg 回收、暂存清理后结束，整批提交前复查 Source；旧资源不受影响。列表不携带图片正文，图片命令按归属返回二进制；前端按需读取并在离开时释放 Blob URL。工作台复用原操作入口。

### 实际验证

本批证据保存在 `.local/wb304/`；截图位于 `output/playwright/wb304-*.png`。

| 层级 / 命令或入口 | 实际结果 |
| --- | --- |
| `.venv/Scripts/python.exe -X utf8 -m pytest tests --basetemp=.local/wb304/pytest-final -q` | PASSED，515 项；`python-full-final.log`。新增 16 项覆盖真实 FFmpeg 像素 / 非 I 帧、350→400 / 1050→1100ms、正 PTS 5350→5400ms、头尾 / 窄范围、缺源、缺 FFmpeg、损坏、超时、横竖 / SAR 和黑帧 |
| `cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib`（工作区 TEMP/TMP） | PASSED，219 项、2 项 ignored；`core-full-final.log`。9 项 keyframe 测试含 Source / 版本绑定、迁移回滚重试、PNG 截断 / IDAT / IEND、恢复关系、原子整批失败、重开及历史源两次备份恢复 |
| 取消 / 退出进程实测 | PASSED，`core-keyframes-r1.log` 记录 `actual_ffmpeg=true`，取消和 Core shutdown 后引擎 / 实际 FFmpeg PID 均不存在、暂存目录 / 登记清除；旧图保留，失败可重试 |
| `npm run build` | PASSED；`frontend-build-r2.log`，既有 bundle 体量提示保留 |
| `cargo clippy --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings` | PASSED；`clippy-final.log`，包括实际二进制命令及新 example |
| `tests/library-keyframes.browser.js` | PASSED，7 组；三视口 1440×900 / 1280×800 / 1024×680，画面 / 时间、预览 Escape、无横溢出、Blob 释放、缺源可读 / 禁提取、取消、失败 / 删除重试、迟到启动 / 事件、初始 null 读取反序与订阅释放。使用合成 IPC，视频时钟来自真实 HTML media |
| `tests/library-keyframes.native.js` | PASSED，真实 WebView2 / Tauri / SQLite / asset 协议 / 打包 engine 0.17.0；正常视频 350→400ms，片段请求625/875/1125→实际700/900/1200ms，正起点 MP4 5350→5400ms、黑帧保留；实际 PNG 二进制 IPC与图片解码，预览 / Escape、无效头部不写入、跨资产读拒绝、删除图片保留片段 |
| `keyframes_acceptance` real Core example | PASSED，源码 / 打包引擎 × 普通竖屏 / 正起点 MP4 共 4 次，配置见 `real-*.json`，各 `real-*/report.json` 留存4张PNG、原始PTS、重开、无视频包恢复及原媒体SHA256不变；不是 GUI 验收 |

原生仅使用 `me.disign.yingqu.p1a-wb304-final-upgrade` 的带标记合成 profile，媒体是本仓库测试样本；没有真实平台下载或新安装包验收。正起点样本 SHA-256 为 `6483321ed5425acfcae8f08a25424e9581b010a9796fa5810815e235de6385be`。既有 `tests/library-clips.browser.js` 独立会话串行回归 9 组通过（`browser-clips-isolated.log`），覆盖三视口、旧版本引用、草稿保护、正起点范围及播放结束。交付检查通过：53 文件的本地 Markdown 引用 241 处、Python AST / JSON、新文件空白、任务总数 49 / 勾选 21 及 `git diff --check`，仅保留既有换行提示。

### 发现、修复与边界

真实 FFmpeg 首轮暴露正起点视频 accurate seek 二次偏移，改用保留 PTS 的 noaccurate_seek；独立审查发现 SAR 丢失导致画面变形，已补非方形像素用例。PNG 初版只检查头部，已采用 image-png 完整解码，补截断 / 校验失败用例；没有新增包版本升级。

原生首轮启动提取触发主线程没有 Tokio reactor 的 panic，同步启动命令改为 async 并重建隔离程序，原生全流程通过。前端审查发现初始 null / 旧操作读取可能晚于 start / cancel 后的查询返回，已将全部操作读取纳入同一请求代次并补反序回归，禁止旧响应清除新操作的取消入口。

首次宽范围并行测试曾出现既有 TDL fixture events.jsonl EOF、独立重跑 TDL_HTTP_ERROR，以及 probe 取消暂存清理 IO_ERROR；未修改 TDL / probe 逻辑或弱化断言。probe 单项通过，随后使用工作区 TEMP/TMP 顺序完成 Rust 全量及 Python 全量均通过。浏览器调试期间的脚本参数、跨 CDP ArrayBuffer 序列化、旧完成快照等脚本错误已修正；一次共用浏览器会话并发导致超时 / beforeunload，丢弃该次结果，重新建立隔离浏览器串行验证。

WB-304 Windows 子项完成；macOS Intel / Apple Silicon 为 NOT_RUN，总项保持未勾选，主任务仍 **21 / 49**。不把 Windows 原生 / Core 测试替代 macOS、全新设备安装、MCP 图片消费或人工完整视听评价。

### 审查与交付核验

使用 code-delivery-review Standard / CHILD_AGENT Tier 3、PARTITIONED_PLUS_INTEGRATION，以本批基线审查。Python / 协议、Rust / 资源、前端分区通过后，独立最终集成完整覆盖 53 文件（分区并集 40 文件，加真实引擎 example 和 12 份文档）。终态 **APPROVED / PASSED / COMPLETE / FRESH（Windows 限定）**，无剩余 P0 / P1 / P2；审查阻断修复两轮：SAR 比例、状态读取乱序，另修复 PNG 完整性检查。

回填前终审身份为 `8d05305f44e330caf8a1726238a24a80672f7838df673fd670809634890f1c86`（`.local/wb304/final-target-r1.json`），各分区清单及最终新鲜度另留于该目录。终审后仅回填本段审查 / 验证元信息，不改变代码、范围或平台边界。`cargo doc --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --no-deps` 通过；既有备份界面回归 4 组通过（`browser-backup-final.log`）。带标记的原生测试程序在空闲状态核验路径后停止，关闭本轮 CLI 浏览器和 Vite，1420 / 9334 零监听；保留合成 profile / 证据（`cleanup.json`）。

## 第二十三批：WB-305 视频与文案同步阅读

### 范围和实现

起始 master / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，本轮开始保存 `.local/workbench-batch23-baseline/` 的 476 文件基线，保留既有 WB-001～WB-304 工作。仅实现同步阅读、段落跳转、播放跟随、自动滚动开关、局部键盘及连续段落片段选取；没有提交、推送、发布、安装工具、使用凭据或访问日常知识库。

新 `library-transcript-reader.tsx` 消费已保存的 timing，按真实播放器时钟二分匹配 `[start,end)`，间隙不高亮。播放器通过来源绑定的句柄提供 seek 与可清理事件订阅，同一视频复用会话，新的跳转取消旧定位，退出释放视频和订阅；沿用实际首帧 / duration / seekable 边界，不重置正 PTS 零点。

自动滚动仅在内部阅读器需要时移动，不抢焦点；关闭保留高亮并允许自由阅读，选取时暂停自动滚动。Space / 上下键 / Enter 仅作用于段落区域，编辑器及对话框保持原键盘行为。范围选择保留连续中间段与间隙，预填原片段表单并绑定文案版本 / 原句 / 文件，保存仍由 Core 验证。来源变化或错文件禁用同步，旧文案、整理稿和笔记不伪造时间；全文引用、复制与导出继续可用。

无 Rust / Python 生产逻辑、Tauri 命令、依赖、schema、备份版本或 MCP 范围变化：schema v6 / backup v4 / engine 0.17.0 保持。契约见[同步阅读](transcript-sync-reading.md)。

### 验证证据

证据位于 `.local/wb305/`；三视口及原生截图位于 `output/playwright/wb305-*.png`。

| 验证 | 实际结果 |
| --- | --- |
| `npm run build` | PASSED，`build-r1.log`；TypeScript / Vite 完成，保留既有 bundle 体量提示。分区初版构建与最后类型检查另有记录 |
| `tests/library-sync.browser.js` | PASSED，9 组；`browser-final-isolated.log`。1440×900、1280×800、1024×680：真实媒体段落定位 / 当前段、段末与静音间隙、Space / 方向键 / Enter、无横溢出；选取模式先播放不误选、可访问名称正文；反向连续选择 / 取消、精确起止 / 版本 / 引用、连续快速 seek 末次请求胜出；60s / 120 段高亮、自动滚动关闭 / 开启且不抢焦点、全文 / 整理稿键盘隔离；错文件 / 变源禁同步；正起点 MP4 5s 零点、头尾拒绝和退出释放 |
| `tests/library-sync.native.js` | PASSED，2 组；`native-sync-r2.log`。真实 Tauri / WebView2 / SQLite / asset protocol，合成短样本 .8s 跳转、1s 高亮、.5s 间隙、Space 播放；原生保存 .2～1.2s 片段，文案 ID / v2 / file ID / 原句准确。297.586s 长样本复用 WB-301 的 79 段真实 CUDA VAD / 文本，200s 后跳转与当前段、自动滚动开关及媒体释放通过 |
| 原生启动 / 类型绑定 | PASSED，`native-build.log` 使用 `p1a-acceptance` 构建生产播放器及命令；仅访问带标记的 `me.disign.yingqu.p1a-wb305-final-upgrade` profile。本轮不是新安装包验收 |
| 现有播放器回归 | PASSED，`video-regression-final.log`，7 组：横竖媒体、seek / 音量、来源和文件切换、导航释放、缺失 / 解码失败、迟到 URL / 详情响应 |
| 现有片段回归 | PASSED，`clips-regression-final.log`，9 组：无时间文案、旧版本原句、草稿保护、文件绑定 / 缺源、动态 PTS 完整范围、提前结束和保存后离开 |
| 现有文案回归 | PASSED，`writing-regression-final.log`，5 组：三视口、原文阅读 / 复制 / 导出、旧模型、版本、编辑保护、笔记和引用 |
| 现有关键帧回归 | PASSED，`keyframes-regression.log`，7 组：采样 / 时间、真实播放时钟、预览 / URL 回收、缺源、取消 / 删除重试和状态读取乱序 |
| 清理 | PASSED，`cleanup.json`：本轮原生程序核验 marker / 路径且空闲后停止，CLI 浏览器 / Vite 结束，1420 / 9335 零监听，保留标记 profile 和证据 |

浏览器使用合成文档与 IPC、真实 HTML media；原生数据由 `seed_sync_profile.py` 仅写标记测试库，Core 实际读取 / 校验，片段通过生产命令保存。约 5 分钟样本是 WB-301 留存测试 WAV 与合成灰色视频复用封装，时间和正文直接复用此前真实 CUDA 结果（79 段，最长 30s），本轮未重新执行 ASR，不据此声明原视频画面逐帧对齐、逐字时间或完整人工视听评价。

首次脚本在 fixture 初始化前调用 seedTiming，后改为等待入口就绪。原生合成库准备曾遗漏识别文档模型字段、重复添加 Windows canonical 前缀及 SQL 参数数量不符，均仅影响新建的带标记 fixture；按既有存储契约补齐后原生来源和读取通过，没有改生产核心规避校验。开发过程在 Vite 监听文件变更后复用旧会话，出现旧表单 / beforeunload 干扰；冻结代码后建立新 CLI 会话串行验证，丢弃受干扰结果，不放宽行为断言。

### 平台和审查

WB-305 的 Windows 实现与验证完成；macOS Intel / Apple Silicon 为 **NOT_RUN**，总项保持未勾选，主任务仍 **21 / 49**，此前 Mac 待办不变。没有把浏览器、Windows 原生、旧 ASR 或编译成功当作跨平台通过。

code-delivery-review Standard / CHILD_AGENT，采用分区加最终集成策略。六个实现文件的冻结身份 `a2c9d7799f99a85003470b3f69fdf4fdb60d789e5058c69529a6d5112100c03e`（`ui-checkpoint-r1.json`）已通过独立完整审查；发现的两项 P2（选取模式未加载时播放误选、可访问名称缺正文）已修复并补验证，无剩余 P0 / P1 / P2。最终独立集成审查完整覆盖 **22 文件**：六个实现、五个测试 / fixture、11 份文档；核验 41 份证据哈希和 476 文件基线，终态 **APPROVED / PASSED / COMPLETE / FRESH（Windows 限定）**，无剩余 P0 / P1 / P2，无阻塞修复轮。回填前终审身份 `118489183f47185cf6e9dcf9a1da983b1ac608cefebd40a17407d8c9ff3233c1`，清单位于 `.local/wb305/final-target.json`，完整本轮差异 / 证据索引为 `final.patch` / `evidence-inventory.json`。终审后仅回填本段审查元信息，代码、契约、验证结果和平台边界未变。

交付检查通过：240 个本地 Markdown 引用、Python AST、新文件空白、`git diff --check`、49 个主任务 / 21 项已勾选；没有扩大到无变化的 Rust / Python 生产核心全量重测。

## 第二十四批：WB-306 结构化文案和制作手法拆解

### 范围与实现

起始 master / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，保存 `.local/workbench-batch24-baseline/` 的 481 文件基线，仅审本轮增量，保留之前所有工作。实现六字段文案模板、画面 / 字幕 / 转场 / 声音的范围观察、资源 / 原文引用、人工 / AI 创建来源及人工修订标识。未提交、推送、发布、安装工具、调用模型、使用凭据或访问日常知识库；没有启动 WB-307、WB-310 或 WB-312。

复用原 asset_notes，schema v7 增加 note_analyses；content 与 body 投影同一事务提交，CAS 冲突和中途失败回滚。正文继续供检索、纯文本导出和原 MCP 只读视图使用，旧自由文本笔记不改写。两项 Tauri 命令经 Core notes 事件接入，不新建服务或调度器。engine 0.17.0 / JSONL v3 不变，无新增依赖。

文案六字段与制作观察分开记录，空字段不猜测内容；观察依据默认待核验。静图依据不能用于声音，转场静图标记未核验连续运动。片段 / 关键帧引用派生于实际不可变记录，校验资产、文件、原 Source 及精确 PTS 区间；图片显示毫秒四舍五入到区间终点时仍按原 PTS 判断。资源删除后保留旧快照，文字可修订，不重新绑定其他文件。

新 AI 记录须显式手动录入并提供模型，origin / model 不可修改；人工修改 AI 笔记由服务端标 human_revised。未读取或读失败的来源 / 资源如实显示未知或未读取，不默认人工 / 已删除。正文复制使用已保存的服务端投影，冲突副本保留全部结构、来源、模型和引用。逻辑包 writer v5 / reader v1～v5，统一校验来源、正文和引用图；排除视频仍保留结构和 PNG。详细契约见[结构化拆解](structured-analysis.md)。

### 实际验证

证据位于 `.local/wb306/`，截图位于 `output/playwright/wb306-*.png`。

| 检查 | 实际结果 |
| --- | --- |
| Rust 库范围 | PASSED，`library-checkpoint.log` 49 项；新分析 9 项含人工 / AI、不可变文案版本、人工修订、原 PTS 边界、缺失 / 删除、错源、原子写入、迁移失败重试、两轮含 / 不含媒体备份及恶意恢复图 |
| 完整 Core 串行 | PASSED，`core-serial.log`，`cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib -- --test-threads=1`，228 passed / 2 ignored。保持既有串行策略及工作区 TEMP/TMP，不更改任何 TDL / 队列断言 |
| 追加 MCP 消费回归 | PASSED，`analysis-mcp-final.log`，分析测试 10 项。完整 Core 后仅新增一项测试，无生产代码变化；真实调用既有 dispatch 验证结构化正文、AI 创建来源、精确引用、Unicode 单标量切片和写入口仍不开放。原全量与本次增量共同覆盖最终测试状态，没有把追加测试说成又跑过一轮全量 |
| 编译 / 静态检查 | PASSED，`check-initial.log`、`clippy-final.log` 全 targets / desktop,mcp / -D warnings、`rustdoc.log`；新增 Rust 文件经 rustfmt，保留既有代码格式。`frontend-final.log` 的 npm build 通过，保留既有 bundle / use-client 提示 |
| Python 样本兼容 | PASSED，`python-fixtures.log`，现有 MCP 验收样本 2 项；MCP / 同步样本仅放宽到已实现 schema v7，不伪造生产数据 |
| `tests/library-analysis.browser.js` | PASSED，6 组，`browser-final-r2.log`。1440×900 / 1280×800 / 1024×680 六字段保存 / 无溢出；AI 模型必填、提交成功回读失败不重复创建且不冒充人工、手改保留来源 / 模型、结构化冲突副本；四维度范围 / 证据、图像声音拒绝、资源读取失败不冒充删除且可重试、PNG 预览 / 范围播放、缺源 / 删除后保留与修订；原文引用打开和失败重试 |
| `tests/library-analysis.native.js` | PASSED，2 组，`native-analysis-final.log`。真实 Tauri / SQLite / WebView2 / engine0.17.0：六字段、显式 AI 手动录入、模型要求、固定文案引用 / 打开、人工修订；真实 .2～1.2s 片段、实际 500ms PNG、四维度依据、图片解码、范围播放、Core 拒绝仅凭图片的声音声明 |
| 空闲原生重开 | PASSED，`native-restart-result.json`。`library-analysis.restart.js` 在空闲停止测试程序前后只读比较 5 篇本轮笔记，ID、origin / model、human_revised、revision、文案 / 片段 / 图片引用、范围及 body 完全一致；这是空闲强制重开，不等于优雅退出测试 |
| 既有前端回归 | PASSED，`writing-regression.log` 5 组、`writing-failures-regression.log` 9 组、`backup-regression.log` 4 组、`clips-regression.log` 9 组，共 27 组；原文 / 旧模型、编辑和回读失败、冲突、草稿导航、备份选择 / 状态、片段动态范围及提前结束行为保持 |
| 清理 | PASSED，`cleanup.json`，仅停止 marker / 路径核验后的空闲 `wb306-final` 原生测试程序，关闭本轮 CLI 浏览器和 Vite，1420 / 9336 零监听；本轮临时目录相关活跃 Python 子进程为 0，保留标记 profile 和证据 |

原生标识为 `me.disign.yingqu.p1a-wb306-final-upgrade`，从既有合成旧库实际迁移到 v7。AI 内容和模型名均是验收时手动输入的合成资料，不是本轮模型推理结果；观察依据为用户声明，不把可引用截图当作已经看完运动或听完声音。本轮没有新安装包、真实平台下载或完整人工视听验收。

首轮 Core 默认高并发出现 19 个既有 TDL 超时 / 队列日志读取竞争（209 passed / 19 failed / 2 ignored），与此前正式验证命令采用的串行模式不一致；核对未改动的相邻测试和既有策略后，按串行完整重跑通过，未弱化断言。浏览器脚本初版遗漏展开冲突副本和 selectionchange，后修正。资源故障回归改用持续注入，避免并发刷新提前消耗单次故障；原生提取脚本改为返回同一操作的终态快照，实际图片已生成，不用瞬时 running 快照判失败。以上受干扰 / 中间结果不计为通过。

### 平台与审查

Windows 子项已完成；macOS Intel / Apple Silicon 为 **NOT_RUN**，WB-306 总项保持未勾选，主任务仍 **21 / 49**，此前 Mac 待办不变。

code-delivery-review Standard / CHILD_AGENT / PARTITIONED_PLUS_INTEGRATION。后端 14 文件分区（backend-r1.json，`a976139d0b4fc93aaba9d3ae4af3ff480232d87a8a959b4672e383eabf3cc5d4`）与前端 8 文件分区（frontend-r2.json，`3939d3d303c073ab2447ebfaca3f44094adf875466d3e30ec34bd6a05d441c90`）均独立审查 PASSED / COMPLETE / FRESH，无 P0 / P1 / P2。审查 P2 资源下拉项误报删除已修复，另主动修正来源未读时的默认人工。最终独立集成审查 **APPROVED / PASSED / COMPLETE / FRESH（Windows 限定）**，完整覆盖 39 文件：后端 14、前端 8、测试 / fixture 5、文档 12；46 项证据哈希匹配，独立核对原生重开 5 篇完整快照，无剩余 P0 / P1 / P2，无阻断修复轮。回填前终审身份 `c67567ed29d68047bcc29e722bf96a0d1eb12c6860dc18a5c3ce5b0a3061eb3e`，目标 / 增量 / 证据分别见 `.local/wb306/final-target.json`、`final.patch`、`evidence-inventory.json`。终审后仅回填本段审查元信息，代码、契约、验证结果和平台边界不变。

交付检查通过：253 个本地 Markdown 路径、新增 5 处本地锚点、Python AST、新文件空白、`git diff --check`、49 个主任务 / 21 项勾选；481 文件基线无范围外变化。

## 第二十五批：WB-307 跨视频对比工作区

### 范围与实现

起始 master / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，`.local/workbench-batch25-baseline/` 保存 491 文件原状态。本批仅实现跨视频比较，保留前批 dirty 工作，未提交、推送、发布、安装工具、使用凭据或日常知识库。WB-308 按用户后续请求单独推进，未混入本批差异。

复用知识库多选、结构化拆解、片段播放器和原笔记。2～4 个稳定资产列按维度对齐，未知不借用其他视频；逐结论勾选时固定来源 pin，后续列切换不改变结论引用。Rust 解析并验证成员、文案、拆解修订及片段身份，正文和结构事务保存；AI 拆解保留原创建来源 / 模型，比较笔记由人工创建。来源读取与对话框按资产 / ID / 版本绑定，旧拆解展示保留摘录并标明当前修订，关闭释放播放器并恢复键盘焦点。

schema v8 增加 note_comparisons，writer v6 / reader v1～v6。普通正文编辑不能绕过结构，旧比较修改只改结论；备份验证图、正文、引用和历史快照形状，媒体可选但关系不丢失。没有新增依赖或模型调用，engine 0.17.0 / JSONL v3 与 MCP 只读白名单不变。详细契约见[跨视频对比](video-comparison.md)。

### 实际验证

证据位于 `.local/wb307/`，截图位于 `output/playwright/wb307-*.png`。

| 检查 | 实际结果 |
| --- | --- |
| Rust 定向与库范围 | PASSED，comparison-tests-final.log 6 项、library-final.log 56 项；未知 / Unicode、外资产 ID、过期修订、成员去重、CAS / 原子失败、旧引用保留、schema 7→8、一致性备份、两轮含 / 不含媒体恢复、旧包字段夹带、畸形历史快照与 MCP 只读正文消费 |
| 完整 Core 串行 | PASSED，core-final.log，235 passed / 2 ignored；工作区 TEMP/TMP，`cargo test --offline --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib -- --test-threads=1` |
| 静态与构建 | PASSED，clippy.log 全 targets / desktop,mcp / -D warnings；rustdoc.log；frontend-final.log 的 npm build，保留已有 bundle / use-client 提示 |
| 浏览器工作区 | PASSED，browser-final.log 7 组：三视口、AI / 模型、未知与空字段、切版不改结论引用、精确原文打开、失败与过期保留草稿、显式来源重选、旧比较编辑保留修订、真实媒体解码 / 范围 / 关闭回收、读取失败不冒充删除、迟到响应、四列局部滚动 / 第五项反馈、提交成功回读失败不重复创建 |
| 键盘 | PASSED，keyboard-final.log；Space 多选、Enter 对比及原文入口、少于两项错误反馈、Escape 关闭并回到触发按钮 |
| Windows 原生 | PASSED，native-final.log 2 组；生产 Tauri / Core / SQLite、两个真实迁移文案分别打开、AI 来源与人工比较区分、错资产 / 过期修订拒绝、结论 CAS 编辑不改变来源 |
| 空闲原生重开 | PASSED，native-restart-result.json；标记 profile 两篇合成比较，重开前后完整 JSON 一致，包括两资产文案 ID / 版本、拆解修订与 AI 模型、正文 / citations。仅空闲强制重开，不替代优雅退出 |
| 既有 UI 回归 | PASSED，writing-regression.log 5、writing-failures-regression.log 9、analysis-regression.log 6、backup-regression.log 4，共 24 组 |

原生标识 `me.disign.yingqu.p1a-wb307-final-upgrade`，实际从合成旧库迁移到 schema8。原生测试仅使用合成资料；两篇比较中一篇来自首轮脚本已完成的合法提交，另一篇完成后续修订，逐篇重开一致。脚本首次在 CLI 进程侧调用不存在的 structuredClone 失败，改为普通 JSON 复制，未修改产品行为。首轮完整回归的唯一失败为旧 WB306 测试模拟 schema6 时仍留有新增 v8 表，已修正模拟并完整重跑；没有放宽生产校验或测试断言。避免同时重建运行中的 Windows 测试 EXE，修复后定向测试已串行通过。

独立审查发现历史快照恢复形状边界缺口（r0、空 / 重复片段 ID、超长摘录、错误字段数），已补齐并加入重算正文后的拒绝测试。额外键盘检查发现手动挂载来源 Dialog 缺少关闭焦点恢复，已修正并通过键盘及工作区全脚本复测。

### 平台与审查

Windows 子项完成；macOS Intel / Apple Silicon 为 **NOT_RUN**，总项保持未勾选，主任务仍 **21 / 49**。没有新安装包、真实平台下载、AI 推理或人工视听评价，本批证据不替代这些验收。

code-delivery-review Standard / CHILD_AGENT 分区审查：后端及 schema 样本 15 文件、前端及 fixture / browser 11 文件已独立覆盖；最终独立集成审查 **APPROVED / PASSED / COMPLETE / FRESH（Windows 限定）**，41 文件完整覆盖，24 项证据哈希匹配，无剩余 P0 / P1 / P2。回填前终审身份 `6d11c804f516e02bcee2aada0989b6b09c568b34a27411cb81f69a7cbe7db45c`，见 `.local/wb307/integration-review.json`、`target.json`、`owned.diff` 和 `evidence-inventory.json`；之后仅回填本段审查元信息。交付检查通过：266 个本地 Markdown 路径、Python AST、空白和 git diff --check，49 个主任务 / 21 项勾选；cleanup.json 确认 1420 / 9337 零监听，仅清理本轮标记程序，保留 profile 与证据。

## 第二十六批：WB-308 研究专题

### 范围与实现

本批由用户在 WB-307 收尾时明确追加；WB-307 完成独立终审 / freshness 后，捕获 `.local/workbench-batch26-baseline/` 的 500 文件基线，保留全部先前 dirty 工作，仅记录本批增量。HEAD 仍为 master / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`，未提交、推送、发布、安装工具、使用凭据或日常资料，没有推进 WB-309 / WB-310。

schema v9 增加 research_topics / research_topic_members，创建和编辑专题，组合视频 / 片段 / 笔记并增删 / 排序。关系独立于来源所有权，移除不删除来源，名称快照保留缺失引用；归档和源文件状态实时解析，片段复用原 clips 的 source_changed，重新关联不冒充旧片段可播放。专题级 CAS、完整成员 ID 全排列和关系 / 修订的单一事务防止覆盖错序与半保存。

知识库入口复用外壳、搜索 / 分页、原播放器和笔记引用阅读；视频成员进入原详情时隐藏专题，返回当前专题。草稿导航保护、来源 Dialog / 键盘焦点、成员操作错误持续可见；资料选择器区分请求中与需回读，冲突允许关闭，提交成功回读失败不重写。逻辑包 writer v7 / reader v1～v7 保存专题和顺序，旧包夹带专题字段拒绝，空专题库也视为非空；备份范围及报告增加 topics 数量。engine 0.17.0 / JSONL v3、MCP 方法 / 工具 / Resources 白名单不变。见[专题契约](research-topics.md)。

### 实际验证

证据位于 `.local/wb308/`，截图位于 `output/playwright/wb308-*.png`。

| 检查 | 实际结果 |
| --- | --- |
| Rust 专题定向 | PASSED，topics-final.log 7 项；三类成员、错误归属 / 去重、全排列与 CAS、原子失败、移除保留来源、缺失 / 归档、重关联后旧片段 changed / 拒播、schema8→9 回滚重试与一致性备份、空专题非空语义、两轮含 / 不含媒体备份恢复 / 顺序和非法包拒绝 |
| 库范围检查点 | PASSED，library-checkpoint.log 62 项；这是来源重关联修复前的检查点，修复后定向 7 项及完整 Core 共同覆盖最终代码，不冒称又跑过一轮相同检查点 |
| 完整 Core 串行 | PASSED，core-final.log，242 passed / 2 ignored；工作区 TEMP/TMP，desktop,mcp、离线、--test-threads=1，没有放宽断言 |
| 构建 / 静态 | PASSED，clippy.log 全 targets / desktop,mcp / -D warnings，rustdoc.log，frontend-final.log 的 npm build；保留已有 bundle / use-client 提示 |
| Python 样本兼容 | PASSED，python-fixtures.log 2 项；仅更新已实现 schema9 的验收样本允许范围 |
| 浏览器 | PASSED，browser-final.log 5 组，1440×900 / 1280×800 / 1024×680：新建 / 三种成员、顺序 / 键盘、视频打开时专题隐藏与返回恢复、原文 / 真实媒体解码 / 关闭回收、移除保护、缺失 / 归档、失败草稿与导航、提交成功回读失败防重写、重复资料和过期添加的 modal 错误 / 可关闭 / 重读后只提交一次 |
| 键盘与搜索 | PASSED，keyboard-search.log；Enter 保存 / 打开、Escape 关闭 picker / 笔记并返回焦点，查询可选最初 20 项以外的 a44 来源 |
| Windows 原生 | PASSED，native-final.log 2 组，真实 Tauri / SQLite / WebView2：专题创建、三类关系和 UI 排序、固定笔记原文、实际 .2～1.2s 播放 / 回收；移除不删 note / clip、归档 / 删除片段仍可见、错 owner / 错排列 / 过期 CAS 不改变专题，实际 backupInfo 返回专题数 |
| 空闲重开 | PASSED，native-restart-result.json；一项原生专题及三成员完整 JSON 在重开前后相同，包括 ID、连续顺序、修订、名称快照及归档 / 缺失；仅空闲强制重开，不替代优雅退出 |
| 既有回归 | PASSED，comparison-regression.log 7 组、backup-regression.log 4 组、writing-failures-regression.log 9 组，共 20 组 |

原生标识为 `me.disign.yingqu.p1a-wb308-final-upgrade`，合成旧库实际迁移到 schema9。首个定向样本删除资产时缺少 library 旧外键指向的 download_tasks 表，补齐测试库前置表后通过；产品迁移不改变。浏览器首轮直接动态 import 未经 Vite 归一化的模块 URL，绕过 fixture 覆盖，改用 fixture 捕获的原 API 引用后通过。

独立后端 P2 片段重新关联状态已通过复用原领域修复。独立前端发现 P1 需回读导致 picker 锁住、P2 错误仅在 modal 后，以及 P1 hidden 被 display:flex 覆盖导致 inactive 专题与详情同时可见，均已修复并实测对应路径；新增可选“返回专题”文案不改变原详情默认“返回列表”。两项前端阻断均已闭环，未越过审查门槛交付。

### 平台与审查

Windows 子项完成；macOS Intel / Apple Silicon 均 **NOT_RUN**，WB-308 总项保持未勾选，主任务继续 **21 / 49**。没有新安装包、真实平台下载、MCP 写入、AI 调用或人工视听评价，本批不替代这些验收。

code-delivery-review Standard / CHILD_AGENT 分区：后端及样本 15 文件、前端 / fixture / browser 12 文件均 PASSED / COMPLETE / FRESH，无剩余 P0 / P1 / P2；最终独立集成审查 **APPROVED / PASSED / COMPLETE / FRESH（Windows 限定）**，43 文件完整覆盖，26 项证据哈希匹配，无剩余 P0 / P1 / P2。回填前终审身份 `600a1452ff3c23f112000db6f42b59bbcb6d547b66ae9134ffb17793d17cc582`，报告 / 目标 / 差异 / 证据见 `.local/wb308/integration-review.json`、`target.json`、`owned.diff`、`evidence-inventory.json`。本段之后仅回填审查元信息，代码、契约、验证和平台边界不变。交付检查通过：280 个本地 Markdown 路径、2 处新增锚点、Python AST、空白及 git diff --check，49 主任务 / 21 勾选；cleanup.json 确认 1420 / 9338 零监听，只停止本轮标记程序与开发服务，保留 profile 和证据。

## 第二十七批：WB-309 自己的选题和草稿

### 范围与实现

用户“继续下一步”按清单顺序推进 WB-309；在实现前捕获 `.local/workbench-batch27-baseline/` 的 511 文件原状态，仅记录本批增量，保留全部之前 dirty 工作。HEAD 仍为 master / `2d32959a7d8367aff57c6a1d962ea048afbc4ea0`。未提交、推送、发布、安装工具、使用凭据或日常知识库，没有推进 WB-310 / MCP 写入。

schema v10 新增 library_ideas / idea_references，自己的标题、观点和正文独立于文案、笔记和专题。完整引用与正文同事务 CAS 保存，新引用由 Rust 验证存在 / 种类 / 资产归属，已有缺失引用按原身份保留。移除仅解除关系，来源和文件不变。视频 / 片段来源状态从 topics 中抽取既有 material_source 逻辑共用，继续使用 clips 原始 Source 判断，不建设另一套状态服务。

工作台提供近期六项 / 总数及入口，摘要不含观点或正文。视频、片段、专题可直接进入仅预选参考的新稿；自己的字段默认空白。已有来源可打开视频详情 / 范围播放器 / 专题及成员，返回原稿不改正文。独立草稿编辑保护、CAS 完整 JSON 副本、异步读取与活跃编辑报告隔离、提交成功回读失败保留 ID / 修订并禁止重复写入。复用现有资料选择器、来源阅读器和设计 token，无新依赖或引擎 / 协议变化。详细契约见[自己的选题与草稿](ideas-and-drafts.md)。

逻辑包 writer v8 / reader v1～v8 增加 ideas / idea_references，恢复验证正文限制、修订、完整关系与连续顺序，允许历史缺失并校验现存来源。只有选题的库也不是空库；备份范围和报告增加 ideas 数量。原 MCP 只读方法 / Tools / Resources 白名单不变，engine 0.17.0 / JSONL v3 不变。

### 实际验证

证据位于 `.local/wb309/`，截图位于 `output/playwright/wb309-*.png`。

| 检查 | 实际结果 |
| --- | --- |
| Rust 选题专项 | PASSED，ideas-checkpoint.log 8 项；原创与原文 / 笔记隔离、三类参考及错误归属 / 类型 / 去重、原子失败 / CAS、移除保护、归档 / 重新关联 / 缺失后继续编辑、有限工作台摘要、空选题非空语义、两轮含 / 不含媒体恢复 / 引用顺序、恶意包与 schema9→10 回滚重试 |
| 库范围 | PASSED，library-checkpoint.log 71 项，覆盖原文、片段 / 来源、结构化拆解、对比与专题相关行为 |
| 完整 Core 串行 | PASSED，core-final.log，250 passed / 2 ignored；工作区 TEMP/TMP、desktop,mcp、离线、--test-threads=1，无测试断言放宽 |
| 静态 / 构建 | PASSED，clippy.log 全 targets / desktop,mcp / -D warnings，rustdoc.log，frontend-final.log 的 npm build；已有 bundle / use-client 提示保留 |
| Python 样本兼容 | PASSED，python-fixtures.log 2 项，仅允许已实现的 schema10 |
| 浏览器 | PASSED，browser-final.log 6 组：三种参考、自己的内容隔离、1440×900 / 1280×800 / 1024×680，视频 / 专题成员往返不改稿、片段真实解码 / 关闭回收、工作台打开准确选题，失败与导航保护、完整冲突副本 / 引用修改、归档与删除后继续编辑、移除保护、提交成功回读失败防重复、三种灵感入口只预选来源 |
| 键盘 / 异步 / 未保存来源往返 | PASSED，keyboard-ordering.log；Enter 入口 / 保存，Escape 关闭资料选择器后返回焦点；先后选择两选题时旧读取迟到不覆盖新选题；有草稿时查看来源可继续编辑或保存后进入，再返回完全相同正文 |
| Windows 原生 | PASSED，native-final.log 2 组；真实 Tauri / SQLite / WebView2 的三类引用和原创保存、灵感入口、来源详情返回、实际 .2～1.2s 范围播放 / 释放、工作台精确目标；移除保留文件、归档 / 已删片段仍可编辑草稿、旧 CAS / 错归属不改变状态、参考文案未被写入改动，实际 backupInfo 计入选题 |
| 空闲重开 | PASSED，native-restart-result.json；一篇选题含观点 / 草稿和两条剩余参考，完整 JSON 在重开前后相同，包括 revision3、原 ID、顺序、名称、缺失 / 归档和专题修订。仅空闲强制重开，不替代优雅退出 |
| 既有回归 | PASSED，topics-regression.log 5 组、backup-regression.log 4 组、writing-failures-regression.log 9 组、workbench-regression.log 4 组，共 22 组 |

原生标识 `me.disign.yingqu.p1a-wb309-final-upgrade`，合成旧库实际迁移到 schema10，未使用日常资料。所有正文、观点和参考标题均为本批手动或测试脚本输入的合成内容，不是模型推理或人工文案质量验收。原生没有新安装包或真实在线平台下载。

### 平台与审查

Windows 子项完成；macOS Intel / Apple Silicon 均为 **NOT_RUN**，WB-309 总项保持未勾选，主任务继续 **21 / 49**。前批的 macOS 待办保持。

code-delivery-review Standard / CHILD_AGENT 分区：后端 / schema 样本 18 文件、前端 / fixture / browser 17 文件已分别 PASSED / COMPLETE / FRESH，无 P0 / P1 / P2；最终独立集成审查 **APPROVED / PASSED / COMPLETE / FRESH（Windows 限定）**，52 文件完整覆盖，27 项证据哈希匹配，无 P0 / P1 / P2。回填前终审身份 `719f7b9393811b91aec23f3e522c47aceabe1d854347646c72d654c420f41fa9`，见 `.local/wb309/integration-review.json`、`target.json`、`owned.diff` 和 `evidence-inventory.json`；终审后仅回填本段审查元信息，代码、契约、实际验证及平台边界保持。交付检查通过：295 个本地 Markdown 路径、2 处新增锚点、Python AST、空白及 git diff --check，49 主任务 / 21 勾选；511 文件基线无范围外变化。cleanup.json 确认 1420 / 9339 零监听，仅停止本轮标记程序和开发服务，保留 profile 与证据。

## 第 28 批：WB-310 受控 MCP 写入

### 范围与实现

用户“继续下一步”按清单推进 WB-310。起点 `.local/workbench-batch28-baseline/` 保存 522 个文件；HEAD 仍为 master / 2d32959a7d8367aff57c6a1d962ea048afbc4ea0，保留此前全部 dirty 工作。没有提交、推送、发布、安装工具、使用真实凭据或日常知识库；没有推进 WB-311 / WB-312。

六项新增工具通过已有 stdio → IPC → Core 领域服务提交。写授权与服务开关分开，均默认关闭；授权校验与提交共用 Core 锁，MCP 没有自行授权、改写旧正文、删除文件或修改设置的入口。仅追加分类 / 标签关系具有幂等性，失败批次回滚。新增笔记使用 AI / model / citations 原字段；专题 / 草稿以 ai_source_json 保存声明的模型与精确文案引用，人工编辑保留创建来源。schema11 / writer9 / reader1～9，迁移先备份、事务失败可恢复；旧包可读，旧版本夹带 AI 元数据拒绝。

设置界面提供“允许 AI 写入”，处理等待、失败和迟到读取。专题 / 草稿展示 AI 创建与可展开引用，复用原有引用的换行及 160px 内滚动约束。详见[受控写入契约](mcp-write-tools.md)。现有依赖和选型未变。

### 验证证据

| 验证 | 结果与证据 |
| --- | --- |
| 新增领域契约 | `write-checkpoint.log`：4 项通过，覆盖六工具、Unicode / 跨资产精确引用、人工 / 原文不变、关系重复与错误回滚、来源保留及包 / 迁移错误 |
| IPC 权限与生命周期 | `ipc-checkpoint.log`：7 项通过；默认未授权、授权 / 撤销、方法白名单、服务重启重置和既有连接回收 |
| 既有库范围 | `library-checkpoint.log`：71 项通过，含历史迁移与文案 / 片段 / 图片 / 拆解 / 比较 / 专题 / 草稿备份关系 |
| 全量核心与补验 | `core-final.log`：254 通过、1 失败、2 忽略；唯一失败是现有 workbench 迁移断言仍期待 schema10。同步期望为11后，`migration-final.log` 定向1项通过。合计255项通过、2项网络测试忽略；没有把首轮全量日志改写为全绿 |
| 静态验证 | `build-final.log`、`clippy-final.log`、`rustdoc-final.log` 通过。Clippy desktop,mcp / all-targets / -D warnings；保留已有 bundle 大小提示 |
| 官方客户端及真实适配进程 | `mcp-client.log` / `run-zYhI9z/summary.json` 通过；六工具默认拒绝、明确授权后写入、精确引用、批次原子性、禁止路由、撤销后仍可读，含原有握手 / EOF / Core 断连 / 进程退出 |
| 读取与 Resources 回归 | `resources-regression.log` 指向 `.local/wb205/run-8X43Ai/summary.json`，读取 / Resources 与凭据路径边界通过；将原先 create_note 未注册的断言同步为合法输入下 WRITE_DISABLED，未放宽读取范围 |
| 浏览器新增 | `browser-writing.log`：6 组通过，含1440×900、1280×800、1024×680、键盘、等待 / 失败 / 迟到状态、AI来源与人工编辑、128字符模型名及长引用换行 / 内滚动；截图 settings-*.png / long-source-1024.png 已查看 |
| 浏览器既有回归 | `browser-settings-regression.log` 5组、`browser-ideas-regression.log` 6组、`browser-topics-regression.log` 5组通过 |
| Windows 原生写入 | `native-write-result.json`：官方 SDK 经真实 stdio / 命名管道写入带 marker 的 wb310-final-upgrade 合成 profile，六工具、原生授权与来源、人工 / 原文保护、撤销 / 服务重开通过 |
| Windows 原生界面 | `native-ui.log`：2组通过，真实开关 / 键盘 / 剪贴板、AI创建专题与草稿展示；截图 native-writing-settings.png / native-writing-idea.png |
| 带授权重启 | 在授权开启、无操作执行时，仅核验并强制停止自有隔离程序后重新启动；`native-restart.json` 证明服务 / 写授权默认关闭，AI note / topic / idea 的完整内容与来源逐字段相等。不据此声称正常退出验收 |
| 测试辅助 | `model-checks.log`：15项只读模型结果审计测试通过；`fixture-tests.log`：9项 profile / packaging 辅助测试通过，没有运行真实模型账号或本批安装器 |

证据目录为 `.local/wb310/`，原生 profile 和程序留在 `.local/p1a-acceptance/wb310-final/upgrade/` 及对应 marker 目录供复查。首次客户端初始化与浏览器冷启动各遇一次宿主超时，读取已有日志确认实际初始化完成后重跑通过；未改动产品超时或放宽业务断言。未运行的 unittest 首次命令显示0测试，随后已改用 pytest 实际执行9项；不把0测试计为通过。

### 交付边界

Windows 实现和上述验证完成。macOS Intel / Apple Silicon 编译、原生权限 / socket / UI 与恢复分别为 NOT_RUN，当前无设备，WB-310 总项保持未勾选。真实模型生成内容质量、多模态、转录长操作及本批安装包未验收。独立核心、UI、剩余适配 / 文档与集成复核记录保存在本批 review 文件中；最终目标与证据摘要以 target.json / evidence-inventory.json 为准。

## 第 29 批：WB-311 MCP 转录长操作

### 范围与实现

用户“继续下一步”按清单执行 WB-311。`.local/workbench-batch29-baseline/` 保存532文件；HEAD仍为 master / 2d32959a7d8367aff57c6a1d962ea048afbc4ea0，保留此前全部脏工作。本轮未提交、推送、发布、安装工具、使用真实凭据或日常资料库，未推进 WB-312。

start_transcription / get_operation / cancel_operation 通过固定 IPC映射复用 Core转录 worker。启动与取消在核心锁内检查WB310写授权；get只读。启动立即返回真实操作 ID，同文件运行复用、不同文件忙碌；取消立即返回running + cancel_requested，实际bridge子进程树回收、暂存清理后才发布canceled。客户端断连、MCP服务停止和请求回执超时不取消Core worker。

操作读取复用active / 失败覆盖 / 每文件最新持久摘要，不新增操作历史表；同文件新操作替换旧摘要后，旧ID明确NOT_FOUND，不别名到新操作。公开DTO无Source、路径、任务ID或正文，沿用固定错误码。缓存结果和新文案版本沿用领域服务，原文 / 整理稿不覆盖。工具目录16项，schema11 / writer9 / reader1～9 / engine0.17.0 / JSONL v3不变，无依赖升级。完整合同见[转录长操作](mcp-transcription-operations.md)。

### 验证证据

本表日志除特别说明外位于 `.local/wb311/`。

| 验证 | 实际结果 |
| --- | --- |
| 核心专项 | operations-checkpoint.log4项通过；夹具UTF-8修复后operations-final.log再次4项通过。覆盖权限、重复 / 单并发、缓存 / force、新旧ID、失败 / 静音 / 重启、诊断脱敏、实际Python引擎与子进程PID退出、暂存目录 / 登记清空 |
| 全量核心 | core-final.log：259通过、0失败、2项在线GitHub测试忽略，串行执行；fixture编码修复后另以transcription-regression.log补验8项既有转录测试全部通过 |
| 静态检查 | compile-initial.log / build.log / clippy.log / rustdoc.log通过；Clippy desktop,mcp all-targets -D warnings。沿用既有bundle体积提示 |
| 官方SDK与真实协议 | official-client.log / run-bAJOPF/summary.json通过。真实stdio和Windows命名管道，16工具Schema；三工具权限、进度、重复 / 忙碌、取消pending、实际引擎 / child退出和stage删除、完成 / 缓存均有断言 |
| 断连与超时 | 启动者关闭后操作继续；测试transport明确捕获Core已提交的启动回执后不交付，官方客户端300ms超时，再调用得到同一ID；关闭 / 重开服务后仍能查询，写授权重置。未改产品5秒IPC超时、worker或业务断言 |
| 旧写工具 / Resources | writes-regression.log指向 `.local/wb310/run-BjeaMS/summary.json`；resources-regression.log指向 `.local/wb205/run-FSfgsv/summary.json`，均通过。旧读取测试将start未注册断言同步为合法参数下WRITE_DISABLED，未放宽读取 / 路径 / 凭据边界 |
| Windows原生控制 | native.log / native-result.json：标记wb311-final-upgrade资料库中，官方SDK经Tauri实际Core完成启动、取消与查询，终态canceled且与桌面摘要一致，撤销后仍可读，既有原文逐字段一致 |
| 原生重启 | 闲置自有测试程序核验路径与marker后强停重开；native-restart.json证明默认服务 / 写授权关闭，原operation ID与完整公开终态DTO相等。不据此声明正常退出或运行中崩溃恢复验收 |
| UI | browser-regression.log6组通过，含1440×900、1280×800、1024×680、键盘、迟到读取 / 失败及已有AI来源交互；native-ui.log1组通过实际原生授权范围说明和键盘。native-settings.png已查看 |
| 辅助测试 | python-tests.log25项通过（转录 / marker / packaging辅助），client-audit.log15项通过；本轮没有执行模型账号、模型质量验收或安装器 |

### 排查与修复边界

官方联测先暴露了测试夹具问题：desktop_engine.py 在Windows默认stdin编码下误读含中文的UTF-8 JSONL路径；夹具现显式使用UTF-8。marker保护的合成host曾把原始路径登记给asset_files，违反既有canonical路径绑定，已改为和真实入库相同的canonicalize；保持产品路径校验。测试fixture首次status冷启动曾超过原8秒，准备阶段上限改30秒，仅影响测试host准备，不修改产品IPC时限。原生脚本一次使用错误的桌面读取命令名，已同步实际get_asset_transcript；启动时增加有界等待CDP就绪。独立复核提出的脚本收尾P2已改为嵌套finally，关闭客户端或服务失败时仍关闭CDP，不吞掉失败。

### 交付与平台边界

Windows上述实现和验证完成，WB-311的Windows子项已勾选。macOS Intel / Apple Silicon编译、socket、原生控制与进程回收分别NOT_RUN，当前无环境，总项保持未勾选，主清单仍21 / 49。识别质量与真实模型、多模态、正常退出 / 活跃崩溃以及本批安装器未验收；此结果不替代WB-312 / WB-313。原生隔离profile和程序保留供复查，测试服务 / 子进程已停止，最终范围与证据哈希及独立分区 / 集成结果见本批target.json、evidence-inventory.json及review记录。
