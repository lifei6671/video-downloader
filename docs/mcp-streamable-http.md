# 本机 Streamable HTTP MCP

映取以桌面进程提供 MCP，默认关闭。用户在系统设置开启后，复制 `http://127.0.0.1:17653/mcp` 给支持 Streamable HTTP 的本机客户端。不再提供 `yingqu-mcp` 命令启动，不自动修改客户端配置。旧 command / args 配置需要切换为 URL。

## 协议与复用

使用[官方 Rust SDK rmcp 3.8.0](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.8.0)的 `StreamableHttpService` 和 axum HTTP 运行时，启用 server、macros、streamable HTTP server/session features。复用既有 20 项工具、Resources、Prompts、固定错误、资料收录范围及媒体内容块，不自行实现 MCP JSON-RPC 或 SSE。

HTTP 监听器与内部 IPC 均由同一个 Core 持有；每个实际工具/资源请求使用独立 IPC 连接。Core 仍是 SQLite、权限、下载、转录和媒体进程的唯一权威，不新增第二个数据库连接或任务系统。纯 SDK 协议请求不读取资料库。

SDK 配置为无状态请求和 JSON 优先响应，兼容 2025-11-25 握手以及 2026-07-28 discovery/请求 metadata。HTTP 请求取消沿 SDK RequestContext 取消 token 传给短音频读取，关闭 IPC 连接后 Core 回收实际 FFmpeg 与临时资源。已提交的转录、持久采样仍沿用原有操作 ID 和任务生命周期，客户端断开不把已提交操作伪装为取消。

## 仅本机边界

- 固定绑定 IPv4 loopback `127.0.0.1:17653`，不绑定 `0.0.0.0`、局域网网卡或远程地址，不接受配置任意监听主机。
- SDK Host allowlist 为实际 `127.0.0.1:<port>`；Origin 存在时仅接受该 HTTP origin，拒绝外站及 `null`，阻断浏览器跨站与 DNS rebinding 路径。未携带 Origin 的本机 MCP 客户端可连接。不启用宽泛 CORS 或代理转发。
- HTTP endpoint 只有 `/mcp`。SDK 流式限制整个 POST body 为 64 KiB，超限为 HTTP 413；内部 IPC 响应仍有 1 MiB 上限，普通/音频有界期限保持原契约。
- 稳定连接地址不包含本机文件路径、Cookie、代理、签名下载地址或外部凭据。它不是用户认证 token；访问范围为本机进程和当前开关授权，不提供局域网/远程模式。

## 开关与权限

开启顺序为启动内部 IPC，再绑定 HTTP；HTTP 端口占用时回滚内部 IPC 并返回 `MCP_HTTP_BIND_FAILED`。重复开启幂等。测试夹具可由 Rust 内部指定端口 0，仍强制 loopback，不占日常端口。

关闭时先清除写权限并取消 HTTP 请求，再关闭内部 IPC、等待实际媒体进程和临时资源回收，最后等待 HTTP 退出。请求体未传完也可取消；HTTP 关闭等待有界。完成停用后旧连接和地址不能读取资料。重启应用默认关闭，重新启用为只读；重新建立连接后使用当前桌面授权。

允许 AI 写入仍由桌面应用控制，不由 URL 参数、MCP 工具或客户端控制。关闭写权限后文本、PNG、短音频读取继续可用，新增笔记/分类、追加分类标签、转录和持久采样受限。未收藏素材及非法资产/文件关系继续按现有规则拒绝。

桌面 DTO 与复制接口见[设置契约](mcp-settings.md)，文件分发见[打包契约](mcp-packaging.md)，逐项实际结果见[HTTP 验证](mcp-http-validation.md)。
