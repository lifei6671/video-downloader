# MCP 设置与客户端配置

对应 [WB-206](workbench-knowledge-base-tasks.md)。Windows 设置界面、原生剪贴板、中文空格路径配置和客户端启动已验证；macOS Intel / Apple Silicon 继续待验收。WB-207 已完成[安装包接入及 Windows 安装后连接验证](mcp-packaging.md)，该证据独立于本页的设置界面验收。

## 使用与状态

系统设置增加 MCP 分类。每次打开应用均默认关闭，不持久化开关；进入设置页只读取状态，不会启动服务。开启复用现有 Core IPC 生命周期，关闭等待监听与连接结束。开关在开启、关闭或复制期间禁用，显示正在开启 / 正在关闭；失败保留错误并重新读取实际状态，不假装操作成功。

当前设置页每 3 秒及窗口重新获得焦点时读取状态，离开 MCP 分类停止轮询。异常监听显示服务异常、移除配置，可重新开启；读取失败显示状态不可用并禁止修改，刷新可重试。迟到的读取结果不能覆盖较新的启停结果。

开关旁说明本机客户端可读取知识库，读取内容可能发送给客户端使用的模型服务；每次启动默认关闭，服务关闭或退出映取会断连。仅支持本机 stdio 客户端，云端客户端不能直接使用此配置。

写入授权独立于服务开关，每次启动及关闭服务后默认关闭；界面提供“允许 AI 写入”，请求期间禁用重复动作，失败回读实际权限，迟到的旧状态不能覆盖权限更新。关闭写入保持连接可读；服务异常时界面不继续显示写入可用。WB-311 增加启动 / 取消转录的授权范围，查询仍只读，见[长操作](mcp-transcription-operations.md)。命令及六工具边界见[受控写入](mcp-write-tools.md)。

## 程序发现与格式

Rust 使用当前主程序的实际位置，查找同目录的 `yingqu-mcp.exe`（Windows）或 `yingqu-mcp`（macOS）。不猜测开发目录，不调用 shell 查找，不将不存在或无法无损转换为字符串的路径写入配置。缺少程序时明确提示、禁止开启和复制；服务已开启时仍可关闭。分发目录契约已通过 WB-207 的 Windows 安装验证，macOS 两种架构继续待验收。

服务运行且适配程序可用时显示两种只读配置，默认 Codex（TOML），另有通用 JSON：

```toml
[mcp_servers.yingqu]
command = "<实际适配程序绝对路径>"
args = ["--endpoint", "<实际本机端点>"]
```

```json
{"mcpServers":{"yingqu":{"command":"<实际适配程序绝对路径>","args":["--endpoint","<实际本机端点>"]}}}
```

示例是字段形状，路径和端点由应用运行状态生成。程序与参数分别序列化，`command` 值不额外套 shell 引号，空格和中文属于路径本身；Windows 反斜杠按格式转义。通用 JSON 采用常见的 mcpServers 结构，具体客户端应使用其支持的配置格式，不承诺所有客户端均接受同一外层结构。

复制由原生剪贴板命令执行，显示成功或持续可见的失败反馈。后端在复制时重新读取服务状态及适配程序位置，不信任前端缓存；服务已关闭或配置不可用时拒绝复制。已复制配置在服务关闭期间不能连接，重新开启后由客户端重新建立连接，不隐式启动应用。

## 桌面命令

保留既有低层 `get_mcp_ipc_status` / `set_mcp_ipc_enabled`。新增产品界面命令：

| 命令 | 参数 | 结果 |
| --- | --- | --- |
| `get_mcp_connection` | 无 | `ConnectionInfo`，只读实际状态 |
| `set_mcp_service_enabled` | `enabled: boolean` | 开启前检查程序存在，返回实际 `ConnectionInfo`；失败返回错误 |
| `set_mcp_write_enabled` | `enabled: boolean` | 返回含 write_enabled 的 ConnectionInfo；服务未运行不能授权 |
| `copy_mcp_client_config` | `format: "codex" 或 "json"` | 当前可用配置写入原生剪贴板，无返回正文 |

`ConnectionInfo` 包含 `write_enabled:boolean`、`status: {enabled, endpoint, error}`、`adapter_path`、`adapter_error` 和 `configs: {codex, json} | null`。关闭时没有配置；缺少程序时 `adapter_error=MCP_ADAPTER_MISSING`，非 Unicode 路径为 `MCP_ADAPTER_PATH_INVALID`。复制不可用返回 `MCP_CONFIG_UNAVAILABLE`，剪贴板写入失败为 `CLIPBOARD_WRITE_FAILED`，IPC 错误沿原有错误码传播。

这些路径与端点仅供桌面本机配置使用，不进入 MCP Tools / Resources 的公开知识库 DTO，不包含 Cookie、代理、认证信息或媒体地址。

## 验证边界

复验入口见[开发文档](development.md#wb-206-mcp-设置与配置验证)，实际记录见[第十六批验收](workbench-knowledge-base-validation.md#第十六批wb-206-mcp-设置与连接配置)。浏览器使用合成状态；Windows 原生程序使用带隔离标记的空库，通过实际 Tauri 命令读取状态和剪贴板。

复制出的两种配置分别由 Python tomllib 解析、官方 TypeScript SDK 启动与读取、Codex CLI 0.162.1 app-server 发现七工具验证。客户端 home 与配置均隔离，未发起模型请求。本页的开发实例证据不替代安装包验证；WB-207 已补充独立 Windows 安装证据，WB-208 已完成 [Windows 模型组合验收](mcp-client-acceptance.md)，全新设备与 macOS 继续待办。
