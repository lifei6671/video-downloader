# MCP 转录长操作

对应 WB-311。沿用 rmcp 3.8.0、Streamable HTTP → 本机 IPC → Core，以及现有单转录 worker / PythonBridge / 暂存提交生命周期；没有新调度器、依赖或数据库格式。schema11、备份 writer9 / reader1～9、engine0.17.0 / JSONL v3 不变。Windows 验证见[第 29 批记录](workbench-knowledge-base-validation.md#第-29-批wb-311-mcp-转录长操作)，Apple Silicon 原生验收与 Unix 回收修复见[第 31 批记录](workbench-knowledge-base-validation.md#第-31-批macos-原生组合验收与取消回收修复)；Intel 无可用机器，保持 UNAVAILABLE。

## 工具与授权

当前目录共16项工具：原七项读取、六项新增写入及本页三项。Resources 范围不变。`start_transcription`、`cancel_operation` 受桌面的“允许 AI 写入”控制，客户端不能自行授权；`get_operation` 是只读工具，撤销写权限后仍可调用。启动授权与创建 worker 共用 Core 锁，取消授权与发送取消信号也在同一锁内检查。

| 工具 | 输入 | 结果与行为 |
| --- | --- | --- |
| `start_transcription` | `{asset_id,file_id,force?:false}` | 立即返回真实 Core operation_id 及状态，不等待识别；只能读取指定资产登记的文件，不接受路径或模型配置 |
| `get_operation` | `{operation_id}` | 按精确 ID 查询转录状态，不返回全文、Source、文件路径、任务 ID、引擎参数或凭据 |
| `cancel_operation` | `{operation_id}` | 请求取消后立即返回；若仍 running，则 cancel_requested=true，需继续查询到终态；已终态的操作保持原状态 |

所有对象拒绝未知字段，ID 非空且不超过128个 UTF-8字节。内部 IPC 固定为 `mcp_start_transcription` / `mcp_get_operation` / `mcp_cancel_operation` 三路，沿用64 KiB请求、1 MiB响应及适配器5秒交换超时。三个控制调用均不等待整段视频完成或进程退出。

`start_transcription` 标为非只读、非幂等；`get_operation` 标为只读且幂等；`cancel_operation` 标为非只读且幂等。均 destructiveHint=false / openWorldHint=false。注解不替代 Core 权限检查。

## 并发、重复与结果保存

- 全应用仅一条转录运行。同一文件仍运行时重复启动，即使 force=true，也返回相同操作 ID；取消尚未完成时同样复用。另一文件返回 TRANSCRIPTION_BUSY，不启动第二个进程。
- force=false 且原已保存结果的 Source 未变化，沿用现有缓存语义：返回保留的操作，不重新识别。该操作可能是保留旧结果的失败 / 取消 / 静音记录；has_result仅表示有已保存结果，不表示本轮识别成功。
- 重新识别使用 force=true。新结果作为新的 recognized 文案版本提交，现有原文、整理稿和笔记保留；失败或取消不发布未提交正文，旧成功结果继续可用。文案读取仍通过现有版本化读取工具完成。
- 操作查询复用 active / 未落盘失败覆盖 / asset_transcriptions 最新摘要。每个文件保留当前或最近一次操作，不增加无限历史；同文件新操作替换旧摘要后，旧 ID 返回 OPERATION_NOT_FOUND，绝不会改为查询或取消新操作。不同文件各自的最近操作保留，终态可跨应用重启查询。

## 状态与取消

公开结果固定为 `{operation_id,asset_id,file_id,state,cancel_requested,phase,backend,completed_segments,total_segments,error_code,has_result,sequence,updated_at}`。

state 为 running / completed / no_speech / failed / canceled / interrupted。phase 只公开 extracting_audio / segmenting / loading_model / recognizing，backend 只公开 cpu / cuda，未知值为空；分段计数使用实际进度，不估造百分比。

取消请求只给现有 worker 发送信号，不立即返回 canceled。Core 继续由 PythonBridge 终止并等待实际子进程树，再清理暂存目录和登记记录；完成这些步骤后才发布 canceled。回收或提交失败返回 failed及固定错误，不伪装取消完成。取消和正常完成竞态保留真实终态，取消已完成的操作不会将其重写成 canceled。桌面原有等待式取消接口不变。Unix 路径先终止私有进程组并等待直接子进程，再在现有五秒清理限额内等待进程组消失；只有 ESRCH 证明组已消失，EPERM 仍继续等待。超限或回收失败不会发布取消完成。

## 超时、断连与重启

转录属于 Core，HTTP 客户端断开、MCP请求取消通知、调用超时或关闭MCP服务均不会自动取消后台转录。重开服务默认只读，可重新连接并凭操作 ID 查询；需要取消时先在桌面重新授权。应用退出继续使用既有 Core 关闭流程，异常重启把未正常结束的 running 摘要标为 interrupted，不伪装 canceled。

启动回执超时意味着提交情况未知，不承诺请求级 exactly-once。保留已获得的 ID；同文件仍运行时，可通过 force=false 的启动调用取回相同 ID。操作已结束且未取得 ID 时先在桌面工作台核对；不要自动 force 重试，因为这可能创建新操作。取消超时后查询原 ID；适配器 IPC 超时会关闭该会话，重新建立客户端连接，不更改后台状态。不要把 MCP transport 的取消通知当成 cancel_operation。

## 错误边界与验证

控制调用仅返回固定分类：INVALID_INPUT、WRITE_DISABLED、ASSET_NOT_FOUND、FILE_NOT_FOUND、FILE_MISSING、DEPENDENCY_BUSY、TRANSCRIPTION_BUSY、OPERATION_NOT_FOUND、SERVICE_UNAVAILABLE、CORE_OPERATION_FAILED。后台失败的 error_code 复用现有引擎固定码和本地转录错误白名单，未知诊断归为 ENGINE_PROTOCOL_ERROR；不透传任意 SQLite / IO 文本或路径。业务错误不关闭有效连接，服务丢失与超时沿用适配器生命周期规则。

测试使用合成媒体 / 引擎和标记资料库，官方 SDK 通过当前 Streamable HTTP 与内部 Windows 命名管道验证三工具、进度、客户端断连、Core已提交后丢回执的超时、服务重连、授权重置、真实引擎与子进程 PID回收、暂存清理，以及已完成结果复用。这些夹具不证明识别质量或多模态支持。Apple Silicon 另经真实打包 CPU ASR、原生授权 / 服务开关、实际 PID / 暂存清理、缓存和重启完成组合验收，见[第 31 批记录](workbench-knowledge-base-validation.md#第-31-批macos-原生组合验收与取消回收修复)；确定性回执超时仍由协议夹具验证。Intel 保持 UNAVAILABLE；WB-312 已实现[片段、PNG 与分析模板](mcp-media-tools.md)，WB-313 组合验收仍待执行。

本轮 HTTP 生命周期复验见[HTTP MCP 验证](mcp-http-validation.md)。原生 Apple Silicon ASR 的历史质量及回收记录保留，不作为当前 HTTP 平台验收。
