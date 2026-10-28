# stdio MCP 适配进程

对应 [WB-203](workbench-knowledge-base-tasks.md)。Windows 已完成独立进程、SDK 握手、Core IPC 映射和退出回收验证；macOS Intel / Apple Silicon 编译及运行仍为 `UNAVAILABLE`。WB-204 已扩展为七项只读工具，WB-205 已接入受限 Resources 并通过 Windows 范围 / 隐私边界验证；WB-206 已接入产品设置，WB-207 已完成[安装包接入及 Windows 安装后连接验证](mcp-packaging.md)。

## 启动与连接

使用 WB-201 选定的[官方 rmcp 3.8.0](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.8.0)，保持 `server` / `macros` / `transport-io` 最小 feature，生产依赖置于可选 Cargo feature `mcp` 下。独立入口为 `yingqu-mcp`，不依赖桌面窗口启动代码：

```powershell
cargo build --locked --manifest-path src-tauri/Cargo.toml --features mcp --bin yingqu-mcp
& 'C:\实际安装路径\yingqu-mcp.exe' --endpoint '<当前 Core 返回的系统端点>'
```

第二条只说明参数形状，不能把占位符直接用于连接；客户端需要运行中的桌面应用已开启 IPC，并使用实际端点。WB-206 已通过 [MCP 设置](mcp-settings.md)生成当前程序目录的真实路径及连接配置，WB-207 已接入安装包分发；不自动修改用户的 AI 客户端配置，本轮未发布版本。

参数固定为 `--endpoint <endpoint>`，不接受 URL、远程管道或任意本地 socket。Windows 只接受 `\\.\pipe\yingqu-mcp-v1-<24 位小写十六进制摘要>`；macOS 只接受当前 UID 的 `/tmp/yingqu-mcp-<uid>/<同长度摘要>.sock`。端点权限及内部协议见 [本机 IPC](mcp-local-ipc.md)。

适配进程先连接 IPC 并验证 `ping` 的服务名和协议版本，然后由 SDK 处理 stdin / stdout。它不打开数据库、不启动主应用、不启用 Core 服务、不创建下载或转录调度器，也不启动其他子进程。一个适配进程固定使用一条 IPC 连接，不自动重连；服务重新开启后，由客户端启动新的适配进程。

## SDK 与工具映射

- MCP 生命周期、能力、Schema 和 JSON-RPC 错误均由 SDK 实现。已验证 `2025-11-25` 的 initialize / initialized，以及 `2026-07-28` 的 discovery / 请求 metadata；不自行实现另一套 MCP 协议。
- 声明七项 Tools 与 WB-205 的受限 Resources；不启用订阅 / 变更通知、Prompts、HTTP、远程连接或模型推理能力。Resources 的 URI 与读取约束见 [资源契约](mcp-resources.md)。
- WB-204 已实现七项只读工具。分页、参数、版本 / 修订引用及错误的完整契约见 [只读工具](mcp-read-tools.md)，Windows 完成验证，macOS 继续待验收。

| MCP tool | Core IPC method | structuredContent |
| --- | --- | --- |
| `search_assets` | `mcp_search_assets` | 资产摘要、命中及版本 / 修订引用、游标 |
| `get_asset` | `mcp_get_asset` | 资产元数据、文件状态、分页文档和笔记摘要 |
| `read_transcript` | `mcp_read_transcript` | 指定版本正文切片、总长度、下一位置及来源范围 |
| `list_categories` | `mcp_list_categories` | categories、直接关联未归档资产数、游标及 total |
| `list_tags` | `mcp_list_tags` | tags、未归档资产数、游标及 total |
| `list_notes` | `mcp_list_notes` | 人工 / AI 笔记摘要、修订号、游标及 total |
| `read_note` | `mcp_read_note` | 正文切片、修订引用及分页来源引用 |

WB-310 增加六项受控写入工具，WB-310 时工具目录共十三项，WB-311 新增[三项转录控制 / 查询](mcp-transcription-operations.md)，当前共十六项；未授权的写调用返回 WRITE_DISABLED，七项读取与 Resources 继续可用，详见[受控写入](mcp-write-tools.md)。

输入使用各工具的 DTO，拒绝额外字段；SDK 同时提供输入 / 输出 Schema、读取 / 写入及幂等注解。结果包含 `structuredContent` 和内容一致的 JSON 文本，只构造明确公开的字段，不透传桌面 DTO 或本机路径。业务执行仍由 Core 的白名单决定，不依赖只读注解阻止写入。

IPC actor 在发送前校验 64 KiB 请求上限，过大参数返回业务错误并保留连接；串行执行请求，为每次交换分配 ID，校验响应版本、ID、未知字段、result / error 互斥和最大 1 MiB 响应，拒绝残缺或无换行的 JSON。最多缓存 32 个待执行调用，每次实际 IPC 交换有 5 秒超时。Windows 仅针对 `ERROR_PIPE_BUSY` 每 25 毫秒等待，整个连接尝试最多 5 秒；其他连接错误立即明确失败。

## 关闭与错误

stdout 仅由 SDK 承载 MCP JSON-RPC，参数帮助和诊断只写 stderr。诊断使用固定错误码，不打印命令行端点、资料正文、远程响应内容、SQL 或底层 SDK 错误。`CORE_READ_FAILED` / `RESPONSE_TOO_LARGE` 通过 SDK 工具错误传播，保留会话；传输、协议或超时失败终止该 IPC 会话及适配进程，不将错误伪装为空列表。

| 情形 | 结果 |
| --- | --- |
| 客户端关闭 stdin，握手前或握手后 | 结束 SDK 会话，回收 IPC，退出码 0；Core 继续运行 |
| 应用未启动或服务已关闭时启动 | stderr `IPC_UNAVAILABLE`，退出码 1，无 MCP 成功握手，不启动应用 |
| 已连接的 Core 关闭服务或退出 | 空闲 IPC 读取检测 EOF / 断连，取消并等待 SDK 会话结束，stderr `IPC_DISCONNECTED`，退出码 1；客户端不必先关闭 stdin |
| IPC 无响应超过 5 秒 | `IPC_TIMEOUT`，退出码 1，待执行调用失败并回收连接 |
| IPC 版本、ID、响应结构或内容类型不符 | `IPC_PROTOCOL_ERROR`，不透传原始响应；启动 ping 或交换错误会终止进程，工具结果 DTO 解码错误通过 SDK 返回错误 |
| 非法端点 | `INVALID_ENDPOINT`，退出码 1，不回显输入值 |
| 缺少参数或参数形状错误 | stderr 固定用法，退出码 2，stdout 为空 |

Tokio 的 stdin 由无法取消的阻塞读取实现，可能使默认 runtime drop 在客户端保持 stdin 打开时等待。独立入口在应用会话和 IPC actor 结束后，以 100 毫秒上限关闭 runtime，避免该读取阻止进程退出，依据 [Tokio stdin 文档](https://docs.rs/tokio/1.53.2/tokio/io/fn.stdin.html)。测试直接验证 Core 断连时 stdin 仍打开且子进程自然退出，不用测试运行器强制终止代替通过。

## 验证

WB-206 已接入[设置与客户端配置](mcp-settings.md)：桌面在主程序同目录查找适配程序，开启后生成实际绝对路径与独立参数数组，提供 Codex TOML / 通用 JSON 和原生复制。WB-207 已补充 Windows 安装包的中文空格目录、配置启动与读取证据，macOS 继续待验收。

复验见 [开发文档](development.md#wb-203-stdio-适配进程验证)，实际证据见[实施记录](workbench-knowledge-base-validation.md)。`scripts/verify-mcp-adapter.mjs` 使用已有官方 TypeScript SDK 1.32.1 和合成 Core fixture，所有子进程使用独立资料库 / home、中文空格路径和精简环境，不使用真实媒体、凭据或 AI 客户端日常配置。

自动协议、操作系统进程验证、真实 AI 客户端、模型总结和安装包是不同验收。当前已有 Windows 核心、安装包和 [WB-208 真实客户端模型总结](mcp-client-acceptance.md)的独立证据，未运行全新设备验收；macOS 两种架构的构建、Unix socket 和原生进程验证继续待办，WB-203 总项保持未勾选。
