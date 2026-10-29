# MCP 接入方式变更

当前使用[仅本机 Streamable HTTP](mcp-streamable-http.md)，不再提供 stdio 命令启动。设置中开启 MCP 后复制连接地址；旧 command / args 客户端配置需改为 URL。

官方 rmcp 3.8.0、工具、Resources、Prompts、Core 数据权威和读写授权继续复用。历史 stdio 验收保存在 Git 历史及原专项记录中，不代表当前 HTTP 版本的验证状态；本轮结果见[HTTP MCP 验证](mcp-http-validation.md)。
