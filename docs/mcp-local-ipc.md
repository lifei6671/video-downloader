# 应用核心本机 IPC

对应 [WB-202](workbench-knowledge-base-tasks.md)。Windows 本机管道与独立进程验证已完成；macOS Intel / Apple Silicon 的 Unix domain socket 实现已落盘，编译和运行均为 `UNAVAILABLE`，继续保留验收。这里的协议只连接应用 Core，不是对外 MCP 协议，不能直接作为 AI 客户端的 MCP 配置。

## 启停与所有权

`Core::open` 总是默认关闭 IPC，不自动创建监听端点。`Core::set_mcp_ipc_enabled(true)` 创建监听器；重复开启保持同一端点，关闭等待监听及已连接会话全部结束。`Core::shutdown` 先标记应用关闭，再停止 IPC，禁止重新开启，随后完成既有 worker 回收。

监听器由同一个 Core 持有，会话仅保留 `Weak<Core>`。白名单请求复用现有检索、受控写入及媒体服务，不另开 SQLite 或创建第二套调度器；引擎只由明确授权的媒体请求触发。此内部 IPC 本身不监听 TCP / HTTP，也不隐式启动主应用；对外 HTTP 由同一 Core 持有的独立传输层负责。

旧桌面公开的 `get_mcp_ipc_status` / `set_mcp_ipc_enabled` 已移除，改由 HTTP 服务统一管理。以下状态仅供 Rust 内部与隔离测试使用：

```json
{"enabled":true,"endpoint":"<实际系统端点>","error":null}
```

关闭时 `enabled=false`、`endpoint=null`。监听任务意外结束时 `error=IPC_SERVICE_FAILED`，不继续声称已启用；停用或再次启用会回收原任务，并明确传播失败。开关只在当前 Core 生命周期内生效，重启恢复关闭。当前由[HTTP 服务](mcp-streamable-http.md)统一启停；每个工具或资源请求使用独立内部 IPC 连接，HTTP 取消会关闭该请求的连接。外部客户端只获得 HTTP URL，不接收 IPC 端点或启动命令。原 IPC 限额、当前用户系统端点权限及媒体回收逻辑保留。


## 系统端点与权限

- Windows：`\\.\pipe\yingqu-mcp-v1-<24 位摘要>`。摘要由当前用户 TokenUser SID 和规范化资料库路径生成，隔离不同用户及不同资料库，不在端点名放入真实路径。使用已锁定的 Tokio 1.53.2 `ServerOptions`，首个实例要求 `first_pipe_instance(true)`，所有实例均拒绝远程客户端。
- Windows 权限：用现有 windows-sys 原生 API 创建受保护 DACL `D:P(A;;GA;;;<当前用户 SID>)`，只有该用户的允许 ACE，不继承默认 Everyone / Anonymous 权限；句柄不继承。测试读取实际系统管道的安全描述符核对 SID、保护标记及唯一 ACE。没有切换到另一个真实用户账户测试，不将 ACL 检查描述成跨账户登录验收。
- macOS：`/tmp/yingqu-mcp-<当前 UID>/<24 位摘要>.sock`，资料库规范化路径仅参与摘要，避免用户目录过长触发 `sockaddr_un` 的路径长度限制。目录必须属于当前有效 UID、权限为 `0700`，socket 为 `0600`。不接受目录符号链接或其他文件类型；已有活动 socket 明确报占用，仅回收属于当前用户且连接被拒绝的失效 socket。关闭时移除本监听器绑定的 socket，保留目录。

选型复用[现有 Tokio 实现](https://github.com/tokio-rs/tokio/blob/tokio-1.53.2/tokio/src/net/windows/named_pipe.rs)，权限依据 [Microsoft 管道访问控制](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)。不新增 IPC 依赖框架，仅增加现有 windows-sys 的 `Win32_Security_Authorization` feature；Cargo.lock 没有变化。相同用户的进程属于本地信任范围，端点摘要不是认证凭据。

## 内部协议 v1

UTF-8 JSON Lines，每条请求与响应以换行结束。单连接按序处理，可发送多次请求；不同连接可独立关闭。请求形状固定，拒绝未知顶层字段：

```json
{"version":1,"id":"request-1","method":"list_categories","params":{}}
```

`id` 为非空字符串，最多 128 个 UTF-8 字节；`params` 可省略，旧 ping / list_categories / list_tags 只允许空对象；mcp_* 方法按[只读工具契约](mcp-read-tools.md)和[受控写入契约](mcp-write-tools.md)校验参数。每条请求上限 64 KiB（含换行），响应上限 1 MiB（含换行），最多同时处理 32 个连接；达到连接数后暂停接受，客户端须按系统管道的忙碌语义处理连接失败。Windows 客户端在短暂 `ERROR_PIPE_BUSY` 时需有界等待，适配进程的策略由 WB-203 实现。EOF 丢弃未完整换行的请求，不执行残缺内容。

以下三个 WB-202 方法保持原有形状，供探测及已有本机调用使用：

| method | result |
| --- | --- |
| `ping` | `{"service":"yingqu-core","protocol_version":1}` |
| `list_categories` | 现有 Core 分类列表：`id`、`parent_id`、`name`、`position` |
| `list_tags` | 现有 Core 标签列表：`id`、`name` |

WB-204 另外明确开放 `mcp_search_assets`、`mcp_get_asset`、`mcp_read_transcript`、`mcp_list_categories`、`mcp_list_tags`、`mcp_list_notes`、`mcp_read_note`，使用独立公开 DTO、分页及领域错误；方法名以外的分发仍被拒绝，不通过任意字符串调用 Core。WB-310 另开放六项 mcp_create_note / mcp_create_category / mcp_add_asset_categories / mcp_add_asset_tags / mcp_create_topic / mcp_create_idea，Core 检查独立写授权；WB-311 另开放 mcp_start_transcription / mcp_get_operation / mcp_cancel_operation，复用现有 worker 及独立写授权，见[长操作](mcp-transcription-operations.md)。没有经 IPC 更改授权的入口。详见 [只读工具](mcp-read-tools.md)。

成功响应与失败响应互斥：

```json
{"version":1,"id":"request-1","result":[]}
{"version":1,"id":"request-1","error":{"code":"METHOD_NOT_ALLOWED"}}
```

| code | 行为 |
| --- | --- |
| `INVALID_REQUEST` | JSON、字段、类型或 ID 非法；无法可靠关联请求时 `id=null` |
| `VERSION_UNSUPPORTED` | 请求的内部协议版本不受支持 |
| `METHOD_NOT_ALLOWED` | 不在明确白名单内，不分发到 Core |
| `INVALID_PARAMS` | 当前方法收到非空对象或其他类型参数 |
| `REQUEST_TOO_LARGE` | 返回错误并关闭该连接，其他会话继续可用 |
| `RESPONSE_TOO_LARGE` | 用失败响应替代超大结果，不伪装成空列表 |
| `SERVICE_UNAVAILABLE` | Core 已释放 |
| `CORE_READ_FAILED` | Core 读取失败；不返回 SQL、文件路径或设置内容 |

普通非法请求返回错误后允许继续发请求；客户端断连只结束该会话。服务停止会关闭监听和所有会话，客户端表现为 EOF 或系统断连错误。Core 关闭后不能重新启用。请求中不接受任意 SQL、文件路径、下载、非白名单转录、设置读取 / 修改或白名单以外的写入操作。

## 验证与边界

复验命令见 [开发文档](development.md#wb-202-本机-ipc-验证)，实际结果见[实施记录](workbench-knowledge-base-validation.md)。Windows 覆盖受保护 DACL、默认关闭、实时 Core 数据、重复启停、端点占用、不同资料库隔离、非法请求、分片 Unicode、大小边界和连接回收，并以独立子进程验证通信及退出断连。

WB-203 已接入 stdio，WB-204 已完成七项只读工具及 Windows 分页 / 引用 / Unicode / 边界验证；WB-205～WB-207 的 Resources、设置、配置与分发，以及 WB-208 的 [Windows 真实客户端组合验收](mcp-client-acceptance.md)均已有独立证据，P1B 仅 Windows 限定通过。macOS 两种架构的编译、权限与 socket 生命周期需要各自原生环境验收；Windows 结果不能替代。

## 有界视听请求

关键帧启动/查询/取消通过现有短调用派发，PNG只读取已保存结果。get_audio_excerpt在独立连接按需解码，限制两路并发、最长15秒媒体、1 MiB响应及35秒适配期限；Core使用25秒桥接期限，其他请求保留原有期限和连接。服务停止会通知音频回收进程后再清理临时目录，同时中断堵塞的响应写入。普通断连不取消已授权的持久化转录/采样；请求所有的临时音频会被取消。详见[媒体契约](mcp-media-tools.md)。
