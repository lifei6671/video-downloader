# MCP 设置与连接地址

当前接入方式为[本机 Streamable HTTP](mcp-streamable-http.md)。设置默认关闭，不检查外部适配程序、不提供命令行或 TOML/JSON 格式切换。

开启后展示实际只读连接地址，默认 `http://127.0.0.1:17653/mcp`，并提供“复制地址”。复制时从 Core 重读当前状态；服务关闭或不可用时返回 `MCP_CONNECTION_UNAVAILABLE`，剪贴板失败返回 `CLIPBOARD_WRITE_FAILED`。界面提供操作中、成功与持续错误反馈；后台刷新不得恢复已关闭的服务。

桌面接口：

- `get_mcp_connection()` → `{status:{enabled,endpoint,error},write_enabled}`，endpoint 仅在 HTTP 服务运行时为连接 URL。
- `set_mcp_service_enabled(enabled)` → 同一状态，开启使用当前 Core，关闭会回收 HTTP 会话、内部 IPC 和临时音频进程，清除写授权。
- `set_mcp_write_enabled(enabled)` → 同一状态，仅由桌面应用授权；服务关闭或重启后只读。
- `copy_mcp_connection_url()` → 将当前 URL 写入系统剪贴板，无格式或任意内容参数。

移除桌面公开的内部 IPC 启停命令，以及旧 `copy_mcp_client_config`。MCP 对外工具无法修改服务或授权、读取设置或凭据。开关沿用既有模型数据使用说明；界面不声称模型已实际接收图片或音频。

构建与浏览器验证不替代原生剪贴板、客户端及安装包验证，详见[验证记录](mcp-http-validation.md)。
