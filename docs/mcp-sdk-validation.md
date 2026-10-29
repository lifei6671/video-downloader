# MCP 协议与 SDK 选型验证

本文为 SDK 选型及历史 stdio 试验记录。产品现已使用[本机 Streamable HTTP](mcp-streamable-http.md)，不再提供命令启动；当前结果见[HTTP 验证](mcp-http-validation.md)。

核验日期：2026-10-28。对应 WB-201，技术选型与 Windows 最小互通验证已完成；macOS Intel / Apple Silicon 的构建与客户端验证仍待办。本文不代表知识库已开放 MCP，也不代表 P1B 阶段验收通过。

## 选型结论

采用 [官方 Rust SDK `rmcp 3.8.0`](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.8.0)，保留“stdio 适配进程 → 本机 IPC → 同一应用 Core”的既定架构。版本精确固定，后续升级重新执行本验证；不跟随 Git main 自动变化。

| 项目 | 核实结果 |
| --- | --- |
| 发布与来源 | crates.io 当前稳定版本 3.8.0，未撤回；2026-10-26 22:03 UTC 发布；包内 VCS 来源 `cd88056ffdc97756fbd98e5a12bbeb41758f0ce8` |
| 许可证 | crate 声明 Apache-2.0；上游整份 LICENSE 还包含 MIT 声明，正式分发时保留完整许可和适用声明 |
| 编译条件 | Rust 最低 1.88，edition 2024；本机 Rust 1.98.0 / Windows x64 MSVC 编译通过 |
| 最小 feature | `default-features = false`，仅 `server`、`macros`、`transport-io`；使用现有 Tokio / Serde 技术栈 |
| 产品依赖影响 | 本批独立 probe crate 有自己的 Cargo.lock，未修改生产 Cargo.toml / Cargo.lock、package.json / package-lock.json |
| 客户端目标 | 已安装 Codex CLI 0.162.1，真实 MCP 客户端发现工具通过；独立配置目录，不开模型 turn |

许可证、MSRV 和 edition 依据[固定版本 manifest](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.8.0/Cargo.toml)与[完整 LICENSE](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.8.0/LICENSE)。生产适配已由 WB-203 接入，WB-207 已补齐[完整分发许可与 Windows 安装验证](mcp-packaging.md)；macOS 继续待办，不把本页实验锁文件直接视为生产依赖审查和分发完成。

| 候选 | 决定与依据 |
| --- | --- |
| 官方 Rust SDK | 采用。与 Rust Core、Tokio、原生二进制分发一致，SDK 处理协议、生命周期、Schema 和工具路由；不另建协议栈 |
| 官方 TypeScript SDK | 作为异语言互通客户端。当前项目已通过 shadcn 锁定并安装 1.32.1 / MIT，无需增加生产 Node 进程；本次 `initialize` 实测为 2025-11-25 |
| 扩展现有 Rust/Python JSON Lines 协议为 MCP | 不采用。现有协议 v3 负责引擎任务，不能替代 MCP 的版本、能力、方法和错误契约；继续保持两个适配边界 |

已找到符合当前栈的官方实现，因此不继续引入第三方服务框架或额外运行时。probe 不包含 HTTP / OAuth transport，也不连接数据库、应用 IPC、媒体或下载引擎。

## 协议与兼容性

官方现行规范是 [2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28)。该版本按请求声明协议、客户端身份与能力，可从 `server/discover` 开始；[版本兼容规则](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)明确区分现代流程与旧式会话握手，不能把 `initialize` 写成所有版本必经的唯一入口。

旧客户端继续使用 [2025-11-25 lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle) 的 `initialize` → `notifications/initialized`。本次直接使用 SDK 的默认双模式服务，不在业务适配代码里重写协商逻辑。

| 协议版本 | 证据与状态 |
| --- | --- |
| 2026-07-28 | PASSED：新进程直接 discovery，随后每次请求携带 `_meta.io.modelcontextprotocol/*`；没有发送 initialize |
| 2025-11-25 | PASSED：官方 TS SDK 1.32.1 真子进程握手、工具发现及调用；线报文核对请求和响应版本相同 |
| 2025-06-18、2025-03-26、2024-11-05 | SDK discovery 声明支持；本批未逐版本验证，不记为通过 |
| 未知版本 | PASSED：现代请求指定 `1900-01-01` 返回 `-32022` 并附支持版本信息 |
| 缺少现代请求元数据 | PASSED：返回 `-32602`，不按成功结果处理 |

stdio 使用 UTF-8 单行 JSON-RPC，日志只能写 stderr，关闭 stdin 是正常退出信号，依据[官方 stdio 规范](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio)。SDK 负责实际解析和分发；验证脚本中的固定 JSON 报文只用于断言，不作为生产协议实现复用。

本次还确认错误有两个层级：不存在的工具返回 JSON-RPC `-32602`；已存在工具的参数类型错误由本版 SDK 返回 `CallToolResult.isError = true`。客户端不能只捕获传输异常，也不能把任何 tools/call 结果都显示成成功。WB-204 需保留这些协议差异，并为 `ASSET_NOT_FOUND` 等领域错误定义稳定映射。

## 可运行样例与实际结果

源文件：

- `tests/mcp-sdk-probe/Cargo.toml` / `Cargo.lock`：精确固定 rmcp 3.8.0，独立于生产依赖图。
- `tests/mcp-sdk-probe/src/main.rs`：唯一工具 `probe_echo`，只回显传入的合成文本；声明只读、幂等、无开放外部交互。
- `scripts/verify-mcp-sdk.mjs`：官方 TS 客户端、新协议固定报文和可选真实 Codex 客户端验证；所有子进程隐藏窗口，超时或失败也进入关闭流程。

| 检查 | 结果 |
| --- | --- |
| `cargo build --locked --manifest-path tests/mcp-sdk-probe/Cargo.toml` | PASSED，Windows x64；独立 debug EXE 约 5.35 MB，大小不代表发布优化后体积 |
| `cargo clippy --locked --manifest-path tests/mcp-sdk-probe/Cargo.toml --all-targets -- -D warnings` | PASSED |
| Rust 格式与 `node --check scripts/verify-mcp-sdk.mjs` | PASSED |
| 官方 TS SDK → Rust SDK | PASSED：握手、唯一工具、JSON Schema 2020-12 输入 / 输出、只读注解、structuredContent 和 text JSON 一致 |
| Unicode / 路径 | PASSED：中文、emoji、`100%_`、换行往返一致；EXE 放在含中文和空格的目录中启动 |
| 参数 / 工具错误 | PASSED：整数代替 string 返回 isError，未知工具返回 RPC 错误，未冒充成功 |
| 现代协议 | PASSED：discovery / tools/list / tools/call / 未知版本 / 缺元数据；stdout 每行均可解析为协议 JSON |
| EOF / 退出 | PASSED：现代 probe 与 Codex app-server 关闭 stdin 后在测试期限内正常退出，未触发强制终止 |
| Codex CLI 0.162.1 | PASSED：app-server 的 `mcpServerStatus/list` 经真实 MCP 客户端发现 `probe_echo`、输入 / 输出 Schema、只读注解和服务端名称 |

Codex 的 app-server 协议只用于驱动已安装客户端的测试，不是产品将要暴露的协议。官方入口依据 [Codex App Server](https://learn.chatgpt.com/docs/app-server)；本机生成的 0.162.1 Schema 进一步确认实际方法。测试没有创建对话或开始 turn，不需要登录、API Key 或模型调用；工具执行由官方 TS 客户端验证。Codex 返回的 `runtimeStatus = null` 不当作持续在线状态，也没有据此声明已在该客户端完成真实资料总结。

运行器给 Codex 单独设置 CODEX_HOME、HOME、USERPROFILE、APPDATA 等目录和 cwd，仅继承启动所需的基础环境变量；不会读取 / 改写日常 Codex 配置。官方 TS transport 会先合并默认环境，因此脚本还显式覆盖其默认用户目录，不能只靠从自定义 env 删除字段来隔离。合成请求及响应、stderr、summary 和隔离配置保存在 `.local/wb201/run-*/`。本轮最终通过证据为 `run-mYfU1S`；先前 `run-B5obzi` 是测试错误层级假设不符的失败记录，已经保留，不计为通过。

不带 `--codex` 时只执行协议测试，summary 将客户端项记为 `NOT_RUN`；不能用其总的协议成功状态替代真实 AI 客户端连接证据。复验命令见[开发文档](development.md#wb-201-mcp-sdk-验证)。

## 后续接入边界

WB-202 / WB-203 保留已有本机 IPC 方案，不让 MCP 进程直接开库、启动主应用或调度任务。本批没有实现命名管道、Unix socket、服务开关、真实资料工具或生产启动配置。

WB-204 从当前 Core 的 `search_library`、`library_asset`、`library_document`、`library_categories`、`library_tags`、`library_notes`、`library_note` 映射只读工具。现有桌面 DTO 含绝对文件路径，不能原样透传给 MCP；必须按方案构造公开摘要及稳定 ID / 文档版本引用。全文切片继续按 Unicode 标量和既定 limit 契约实现，本样例不提前实现业务分页、截断或资料权限。

Resources、媒体能力、取消长任务、安装包分发和模型总结质量没有在 echo 样例中验证。只读注解是客户端提示，业务只读边界仍由白名单和 Core 操作决定。P1B 不启用 HTTP、远程 MCP、模型 sampling 或自动写回。

## 平台结论

- Windows x64：SDK 编译、stdio 双模式、跨 SDK 调用、Codex CLI 工具发现为 PASSED。
- macOS Intel：`UNAVAILABLE`，无可用主机，SDK 构建及目标客户端验证保留待办。
- macOS Apple Silicon：`UNAVAILABLE`，无可用主机，SDK 构建及目标客户端验证保留待办。
- 本批 SDK probe 的安装包、真实知识库资料读取和模型总结为 `NOT_RUN`；后续 WB-207 已完成 Windows 安装，WB-208 已完成 [Windows 合成知识库与真实客户端模型验收](mcp-client-acceptance.md)，不回溯替代本批证据。

WB-119 的 macOS 验收状态不变。由于 WB-201 原验收还包含 macOS，任务总项保持未勾选，并单独标记已完成的技术选型与 Windows 验证。
