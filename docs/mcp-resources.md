# 受限 MCP Resources

对应 [WB-205](workbench-knowledge-base-tasks.md)。已实现三类资源 URI、模板发现和统一读取边界，Windows SDK / stdio 验证通过；macOS Intel / Apple Silicon 编译与原生验收继续 `UNAVAILABLE`。WB-206 已接入 [MCP 设置](mcp-settings.md)，WB-207 已完成[安装包接入及 Windows 安装后连接验证](mcp-packaging.md)，WB-208 已完成 [Windows 真实客户端与模型组合验收](mcp-client-acceptance.md)，macOS 继续待办。

## 发现与 URI

SDK 声明 Resources 能力，不启用订阅、变更通知或二进制媒体读取。`resources/templates/list` 返回以下三个 RFC 6570 模板，MIME 均为 `application/json`：

```text
yingqu://assets/{asset_id}{?document_cursor,limit}
yingqu://assets/{asset_id}/documents/{document_id}{?version,offset,length}
yingqu://assets/{asset_id}/notes/{note_id}{?revision,offset,length,citation_offset,citation_limit}
```

`resources/list` 返回空目录，不自动枚举或上传整个资料库；这不表示资料库为空。具体 ID 从现有搜索、资产、文档和笔记工具或已知引用取得，再按模板构造 URI。静态模板和目录均一页返回、没有下一游标；传入 cursor 明确失败，不伪造分页结果。

资产 / 文档 / 笔记 ID 是逻辑标识，不映射为文件路径。路径段采用规范的 UTF-8 百分编码：只允许 RFC 3986 的未保留字符直接出现，其余字符使用大写十六进制转义。例如 ID `中文 +%😀` 的路径段由 `encodeURIComponent` 对这组字符编码后使用。拒绝畸形百分编码、无效 UTF-8、非规范转义、会被 URL 解析器隐式规范化的路径、片段、认证信息、端口以及其他 scheme / 主机。

查询字段按模板白名单读取，不能用查询参数覆盖路径中的 asset_id / document_id / note_id。重复字段、未知字段、空值、负数、小数、非数字或超出整数类型范围的数值均拒绝；document_cursor 是唯一的字符串查询值。URI 总长度不能超过既有 64 KiB IPC 请求预算，参数仍受原有工具限制。

示例读取旧文档中的 Unicode 范围：

```json
{"method":"resources/read","params":{"uri":"yingqu://assets/asset-a/documents/doc-a1?version=1&offset=1&length=3"}}
```

## 与 Tools 共用范围

| Resource | 完全复用的工具读取 | 范围 |
| --- | --- | --- |
| 资产 URI | `get_asset` | 相同资产元数据、文件状态、文档分页和笔记摘要首屏 |
| 文档 URI | `read_transcript` | 必须指定文档 ID，保留资产归属和期望版本检查，默认 4000 / 最大 16000 Unicode 标量 |
| 笔记 URI | `read_note` | 路径 asset_id 必须与笔记归属一致，保留 revision、正文范围和引用分页 |

Resource 直接调用原 Adapter 的同一个类型化读取函数，经已有 IPC 方法进入同一 Core；没有新增直接开库、任意文件读取、网络抓取或第二套权限规则。公开 DTO、错误、列表默认 20 / 最大 100、正文切片和引用摘录限制均复用 [七项只读工具契约](mcp-read-tools.md)。归档仍是组织状态，按 ID 读取与工具一致。

`resources/read` 返回一个 `contents` 项：uri 为本次规范请求 URI，mimeType 为 application/json，text 是对应工具 structuredContent 的 JSON 序列化。没有 blob，也不返回 Source、文件路径、签名媒体地址、设置或凭据。文档 ID 稳定指向不可变版本；笔记 URI 标识笔记本身，跨次读取应附 revision，防止编辑后混用正文。现代协议的保守缓存提示由 SDK 提供，当前 ttlMs=0；不承诺可缓存旧笔记内容。

内部 IPC 响应继续受 1 MiB 上限约束，超限明确失败；此上限不等于外部 MCP 封包的字节上限。Resources 不绕过工具的分页、版本、Unicode 或大小检查，不通过省略参数返回无限全文或全量引用。

## 错误与兼容性

- URI / 查询语法不受支持：RPC `-32602`，固定消息 `INVALID_RESOURCE_URI`；静态目录 / 模板收到 cursor：`INVALID_RESOURCE_CURSOR`。
- 资产、文档或笔记不存在，或归属不符：RPC `-32002`，消息仍为对应 `ASSET_NOT_FOUND` / `DOCUMENT_NOT_FOUND` / `NOTE_NOT_FOUND`。按 [MCP Resources 规范](https://modelcontextprotocol.io/specification/2025-11-25/server/resources#error-handling)采用资源缺失错误码；Tools 保持原有 `-32602`，可读范围与领域语义相同。
- 超限、越界、游标、版本 / 修订冲突等保留 Tools 的领域消息和 RPC `-32602`；内部读取 / 超大响应为 `-32603`。错误不附带请求 URI、SQL、路径或底层诊断，业务失败后仍可继续读取。
- 服务关闭和 Core 退出仍由现有 stdio / IPC 生命周期结束会话，不在资源层重新连接、启动应用或恢复写入。

七项 Tools 保持独立可用。客户端可以完全不调用 Resources，通过搜索、资产详情、文稿、笔记和引用完成相同读取流程；资源能力不会成为工具调用的前置条件。

## 验证

复验见 [开发文档](development.md#wb-205-resources-与边界验证)。隔离库中放入合成 Cookie、代理密码、内部路径和签名地址作为哨兵，只验证存在标志，不使用系统凭据存储或真实用户数据。

官方 TS 客户端经真实 stdio / Windows 管道完成 12 对成功资源 / 工具结果深比较、14 对同域失败和 18 项非法 URI 检查；覆盖分页、旧版本、修订冲突、跨资产读取、Unicode、超限和大小边界。服务端线报文检查不含哨兵值；只使用 Tools 的另一轮运行完成七项读取，报文中没有 resources/* 请求。现代 discovery、旧握手、关闭及自然进程回收也继续验证。
