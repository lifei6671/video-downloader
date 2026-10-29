# MCP 真实客户端与 P1B 验收

当前轻量素材库验收见[本轮记录](material-library-validation.md)。历史 --p2 选题写回依赖已退出公开 MCP 的 create_idea，现于启动前明确拒绝；原记录仅保留当时结果，不作为当前可执行验收入口。当前使用 --materials 验证检索、文案、采样、图片及音频能力边界。

对应 [WB-208](workbench-knowledge-base-tasks.md)。Windows x64 的安装后真实客户端、模型对比、来源核对和断连重连通过；macOS Intel / Apple Silicon 均为 `UNAVAILABLE`，P1B 总项保留未勾选。未发布版本。

## 客户端与资料范围

2026-10-28 使用已安装的 **Codex CLI 0.162.1**，通过其 app-server 驱动真实 stdio MCP 客户端，模型为当前配置返回的 **gpt-6.1-sol**。用户已明确允许使用本机登录，仅将本轮合成文案和人工笔记交给客户端模型。模型推理由外部客户端提供，本机转录不等于模型总结在本地执行。

复用现有 NSIS 构建和安装检查器，独立标识 `me.disign.yingqu.wb208`、产品 `Yingqu WB208`、主程序 `yingqu-wb208.exe`。实际程序位于 `.local/wb207-install/wb208/安装 Program Files/`，前端为安装包内嵌页面。资料库有匹配的 `wb208-acceptance.json` 标记，构造器拒绝未标记、schema 不匹配或已有资产的目录。

样本包含两条目标视频资料、一条其他分类 / 平台资料和一条归档资料。A 属父分类“创作学习”，B 属其子分类“开场钩子”，两者均有“WB208验收”标签和抖音平台；父分类筛选同时返回自身与后代。标题包含“拖延”，两篇正文均包含“任务”：协议测试使用 query“任务”，模型使用分类、标签、平台和未归档组合筛选，不带 query。A 同时保留旧整理稿 v1 与当前 v2，B 为 v1，人工笔记均为 r1。两份媒体使用同一个现有合成占位视频，文案和制作观察为人工构造，不能据此声称模型分析过真实画面。

## 可复验入口与客户端隔离

新增 `tests/fixtures/seed_mcp_acceptance.py`、`scripts/verify-mcp-client.mjs` 和独立断言模块 `scripts/mcp-client-checks.mjs`；不改变产品 Rust、UI、协议、运行依赖或日常 Codex 配置。调用步骤见[开发文档](development.md#wb-208-真实客户端与模型验收)。

运行器按 [App Server](https://learn.chatgpt.com/docs/app-server) 接口和本机生成的 0.162.1 JSON Schema 调用 `thread/start`、`mcpServer/tool/call`、`turn/start`。配置语义参考[官方配置说明](https://learn.chatgpt.com/docs/config-file/config-reference)。只在显式 `--allow-account` 时沿用 Codex 自己的登录存储；不读取、复制或输出令牌，不保存认证客户端的原始 stderr / 完整配置。持久证据仅包含已知合成工具结果、模型报告、状态和 stderr 哈希。

客户端使用临时 cwd、ephemeral 会话、只读 sandbox，关闭 shell、浏览、插件、apps、hooks、skills、memories 和 multi-agent。有效配置按 `config/read` 响应立即缩减为服务器名称列表，为每个会话显式禁用继承服务器，再加入唯一 yingqu；本次禁用 7 个继承项，工具清单仅 yingqu 非空。不能假设 `-c mcp_servers={...}` 会替换已有配置，其行为是合并。模型事件另设白名单，发现命令、文件、图片、子代理或其他服务器调用即失败。

协议操作会话与模型会话分开，模型必须自己发现分类标签、分页检索、读取正文及笔记。自动引用核验只信任成功完成且实际返回内容的工具调用，校验资产、稳定文档 ID、版本、Unicode 标量范围、原句和笔记修订号；失败响应、缺响应、越过返回范围的引文都不能算成功。内容质量需另行审阅，不能从 JSON Schema 合法推断。

## Windows 实测结果

最终完整记录为 `.local/wb208/client-ruGfcM/summary.json`；不以先前中断或隔离配置失败的运行代替。

| 验收 | 结果与证据 |
| --- | --- |
| 协议和七工具 | PASSED；真实 Codex 客户端发现并调用七工具，组合 category / tag / platform / unarchived 筛选，limit=1 翻页仅得到 A / B；16 标量切片、空结果和五类错误边界通过 |
| 实际模型读取 | PASSED；模型自己调用全部七工具，读取 A 的 `wb208-doc-a2` v2、B 的 `wb208-doc-b1` v1、两条 r1 笔记，没有使用旧稿或排除资产；见 `model-tools.json` |
| 对比内容 | PASSED；报告正确区分 A 的时间反差提问、计时器到清单与两分钟行动，B 的左右对照、口播对应勾选与任务改写；给出共同策略、四处差异和三项带来源的复用建议；见 `comparison.json`、`.local/wb208/quality-review.json` |
| 来源引用 | PASSED；四处原句与真实成功工具响应一致；两篇文案和笔记在原生界面核对，引用原文对话框分别显示整理稿 v2 / v1；见 `.local/wb208/ui-reference-proof.json` 及 `source-*-ui.txt`、`note-*-ui.txt`、`pinned-citation-*-ui.txt` |
| 只读和隐私范围 | PASSED；协议与模型阶段前后 23 张逻辑表的行数和内容哈希完全相同；公开 DTO 不含私有字段或路径 / URL 哨兵。见 `.local/wb208/readonly-proof.json`、`database-final-before.json`、`database-final-after-model.json` |
| 关闭服务 | PASSED；原连接调用返回 `-32603 / Transport closed`，新会话明确失败于 required MCP 初始化；不把通用参数错误当断连成功 |
| 服务重连 | PASSED；重新开启后新建会话，按固定 A v2 引用读取成功。已断开的旧 transport 不会自动复活 |
| 应用丢失及重启 | PASSED；终止本轮主进程后旧连接和新会话均明确失败；普通原生重启默认关闭，新会话仍失败；从设置开启后新会话成功重连。此处为故障退出验证，不描述成正常托盘退出 |
| 客户端回收 | PASSED；Codex 关闭 stdin 后自然退出 0，`forced=false`；本轮安装路径下 helper 进程为 0 |
| 核心回归 | PASSED；`cargo test --locked --manifest-path src-tauri/Cargo.toml --features mcp --lib -- --test-threads=1`，196 passed / 2 既有在线检查 ignored |
| 验收脚本回归 | PASSED；Node 15 项、Python 2 项，包含失败响应冒充引用、错误版本 / 返回范围、客户端动作白名单和断连错误语义 |
| macOS Intel / Apple Silicon | UNAVAILABLE；各自保留安装、客户端、Unix socket、原生来源核对和生命周期待办 |

模型没有获得帧、时间戳、播放量或留存数据，报告也明确说明未观看原视频，不能核实实际剪辑节奏或判断哪条效果更好。此次是合成样本的文本学习能力验收，不是多模态、人类视听评价、全新设备或真实平台下载验收。

## 验收过程中的修正与证据边界

- 首次非 PTY 运行虽然完成模型输出，但生命周期输入已结束，记录为 `ABORTED_HARNESS`，不计完整通过；最终在交互终端中完成全部控制步骤。
- 两次客户端配置试运行在模型启动前发现继承服务器或 yingqu 被覆盖，均失败退出；修正为逐会话显式禁用继承项后再完整运行。未改写用户配置文件。
- 初次逻辑快照发现原生界面访问其他资产产生 `asset_visits`，该行为属于明确 UI 访问入口。保留记录，最终模型阶段隐藏测试窗口并观察实际 Tauri 命令，没有 visit 调用，23 表一致；后续人工来源导航在快照之后单独进行，不能混为 MCP 写入。
- 自动审批拒绝带远程调试参数的重启命令，未绕过执行；改用普通启动和原生 Computer Use 完成重启、默认关闭和开启重连验证。
- 既有安装界面检查器硬编码了 `.local/wb207/installed-settings.png`，本次复用覆盖了该旧截图，已将当前图片归档为 `.local/wb208/setup-settings.png`；不把旧路径图片继续当作 WB-207 原图。其余 WB-207 记录和产品文件未改。

完整批次结果及审查见[第十八批实施记录](workbench-knowledge-base-validation.md#第十八批wb-208-真实-mcp-客户端与-p1b-验收)。Windows 限定验收通过不改变 macOS 两种架构待办，也不自动开放 P2 写入或多模态能力。
