# 技术文档

- [轻量素材库与 MCP 视听验收](material-library-validation.md)：主动收藏、范围采样、短音频、收录权限、实际验证与平台缺口。

- [片段收藏与范围播放](asset-clips.md)：WB-303 的版本 / 文件绑定、特殊 MP4 可播放范围、schema v5 与备份 v3，以及 Windows / macOS 验收边界。

- [自媒体工作台、视频知识库与 MCP 技术方案](workbench-knowledge-base-plan.md)：独立资产与文案、分类标签、迁移、MCP 接入及分阶段验收，按阶段实施。
- [工作台与知识库实施任务清单](workbench-knowledge-base-tasks.md)：按依赖拆解的可勾选任务、交付物与验收条件。
- [真实使用问题与优先级评估](workbench-real-use-assessment.md)：WB-401 的用户选题反馈、四步当前原生证据、优先级 / 成本及后续验证目标，评估不代表功能实现。
- [工作台与知识库实施记录](workbench-knowledge-base-validation.md)：调用路径、隔离验证、平台边界和分批开发证据。
- [MCP 协议与 SDK 选型验证](mcp-sdk-validation.md)：WB-201 的固定 SDK、双模式 stdio、Windows 客户端连接、复验入口与 macOS 待办。
- [应用核心本机 IPC](mcp-local-ipc.md)：WB-202 的启停、命名管道 / Unix socket 权限、内部协议 v1、只读白名单与验证边界。
- [本机 Streamable HTTP MCP](mcp-streamable-http.md)：固定连接地址、开关及读写授权、SDK 传输、关闭和媒体回收。
- [HTTP MCP 验证](mcp-http-validation.md)：本轮 Core、SDK、浏览器和原生验证的实际状态。
- [P1B 七项只读工具](mcp-read-tools.md)：WB-204 的公开 Schema、分页、Unicode 切片、版本 / 修订引用、错误及读取边界。
- [受限 MCP Resources](mcp-resources.md)：WB-205 的稳定 URI、模板发现、统一读取范围、路径 / 凭据边界及 Tools 兼容路径。
- [MCP 转录长操作](mcp-transcription-operations.md)：WB-311 的启动 / 查询 / 取消、单并发、权限、断连超时及实际进程回收。
- [MCP 片段、图片与分析模板](mcp-media-tools.md)：WB-312 的固定片段引用、PNG / PTS / 采样范围、三个可选模板及真实客户端媒体边界。
- [MCP 受控写入](mcp-write-tools.md)：WB-310 六项新增工具、独立写入授权、AI 来源、schema11 / backup9 与人工内容保护。
- [MCP 设置与客户端配置](mcp-settings.md)：WB-206 的默认关闭、真实启停、实际程序路径、Codex TOML / 通用 JSON、原生复制及 Windows 验证。
- [MCP 安装包与分发](mcp-packaging.md)：WB-207 的构建 feature、完整许可、安装目录、Windows NSIS 安装后连接与 macOS 待办。
- [MCP 真实客户端与 P1B 验收](mcp-client-acceptance.md)：WB-208 的 Codex / 模型对比、四处来源、只读、断连重连及 Windows 限定结论。
- [文案时间来源与精度验证](transcript-timing-validation.md)：WB-301 的真实 VAD / ASR、语音段精度、音频原点偏移、后续方案及 macOS 待办。
- [可追溯语音段时间](transcript-segments.md)：WB-302 的 PTS 零点、私有时间产物、schema v4、文档 / 文件绑定、失败恢复和备份 v2。
- [文案与制作手法拆解](structured-analysis.md)：WB-306 的六字段模板、四维度范围笔记、关键帧引用、人工 / AI 来源、schema v7 与备份 v5。
- [视频与文案同步阅读](transcript-sync-reading.md)：WB-305 的段落跳转、跟随 / 自动滚动、键盘、连续片段选取、来源绑定及 Windows 验证。
- [关键帧提取与资源管理](asset-keyframes.md)：WB-304 的真实 PTS、均匀采样、PNG 资源、取消回收、schema v6 / 备份 v4 与平台边界。
- [知识库检索与桌面命令契约](workbench-knowledge-base-api.md)：检索规则、强类型命令、摘要 / 全文、错误及变更事件，以及单篇导出、逻辑包与原生备份恢复命令。
- [开发、验证与发布](development.md)：源码运行、测试、引擎打包、CI 与标签发布。
- [单视频文案提取](fun-asr-nano.md)：Fun-ASR-Nano CPU / Windows CUDA、模型按需下载、旧结果保留与导出契约。
- [文案提取验证](nano-cuda-validation.md)：Nano Windows 真实模型、Tauri、回归与跨平台验收边界。
- [macOS 验证记录](macos-validation.md)：Apple Silicon 安装包、双平台 GUI 下载及正常重启恢复，以及 Intel / 旧系统验收边界。
- [解析缓存验收](parse-cache-validation.md)：SQLite 元数据/图片缓存、链接别名、重启预览及下载重新解析。
- [双引擎与图集验收](dual-engine-validation.md)：正式 macOS GUI、文件校验、重启及平台验收边界。
- [DTK 双解析引擎方案与试验](dtk-dual-engine-plan.md)：正式底层切片、导入补丁、图集任务契约和上游升级流程。

- [技术方案](tauri-desktop-technical-plan.md)：双引擎架构、协议、阶段与验收标准。
- [设置与封面验收](m2-cover-settings-validation.md)：Cookie 多行回显、等高设置卡、并发下拉与本地封面。
- [M2 重试与历史管理验收](m2-history-validation.md)：新任务重试、仅移除记录、二次确认删除文件。
- [M2 并发与队列验收](m2-queue-validation.md)：默认 2、可设 1–3、FIFO 调度与取消隔离。
- [M2 第一张卡验收](m2-validation.md)：shadcn/ui 亮色桌面、SQLite、双平台真实单任务闭环。
- [当前 M1 验证](m1-validation.md)：迁移、测试、双平台真实结果及未通过项。
- [旧 M1 历史记录](m1-legacy-validation.md)：旧引擎结果，不计入当前验收。
- [独立 yt-dlp 试验记录](ytdlp-validation.md)：迁移前的 TikTok 下载证据。

用户已批准进入 M2，已完成单任务闭环、并发等待队列及重试/历史管理；全新无 Python Windows 验收移至 M3、仍待验证。启动和构建见[开发文档](development.md)。

- [视频规格选择与抖音 4K 验收](video-format-validation.md)
- [TikTok 视频规格选择在线验收](tiktok-video-format-validation.md)

- [Fun-ASR-Nano Windows CUDA](cuda-acceleration.md)：独立 VAD、版本选择与下载边界；[本轮验证](nano-cuda-validation.md)。

- [自己的选题与草稿](ideas-and-drafts.md)：原创正文、三类参考、灵感入口、来源往返及备份恢复。
- [研究专题](research-topics.md)：三类资料引用、成员增删与排序、缺失保留及备份恢复。
- [跨视频对比](video-comparison.md)：多选资产、逐结论固定来源及比较笔记。
