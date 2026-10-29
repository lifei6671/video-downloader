# MCP 受控写入

对应 WB-310。复用已选定的 rmcp 3.8.0、stdio 适配器、本机 IPC 与 Rust 领域服务，不新增依赖。读取与 Resources 继续使用现有边界；WB-311 已加入[转录长操作](mcp-transcription-operations.md)，WB-312 已加入[片段、PNG 与分析模板](mcp-media-tools.md)。

## 授权与生命周期

应用每次启动时 MCP 服务和写入权限均关闭。先开启服务，再由用户在设置中打开“允许 AI 写入”。此授权作用于当前资料库的本机 MCP 客户端，不按客户端分别授权，不写入设置或客户端配置。

`get_mcp_connection` 增加 `write_enabled:boolean`；桌面命令 `set_mcp_write_enabled(enabled)` 返回真实 ConnectionInfo。服务未运行时不能授权，返回 `MCP_SERVICE_DISABLED`。关闭写入保持连接及读取可用；关闭服务、退出或重新启动应用清除授权，重开服务仍为只读。幂等开启已经运行的服务不撤销既有授权。

WB-310 的工具清单包含七项读取和六项新增写入；WB-311 另加入三项转录控制 / 查询，WB-312 加入两项媒体读取，当前共 18 项。下述六项新增写入可被发现，便于已连接客户端直接响应授权变化。写工具可被发现不代表有权限：Core 在同一个锁内检查权限、关闭状态并提交领域操作。撤销完成后才到达的请求返回 `WRITE_DISABLED`；已提交的数据保留。IPC 没有授权、设置修改、文件删除或任意 Core 调用入口，客户端不能自行打开写入。

## 六项工具

所有对象拒绝未知字段，输入不接受目标文件路径、已有正文 ID 或 SQL。每条内部请求沿用 64 KiB 总上限（含封装与换行），输出沿用 1 MiB。超过限制不执行操作。模型名称是调用方声明的信息，不表示应用已验证模型身份。

`source` 为 `{model_name,citations:[{asset_id,document_id,quote}]}`。model_name 非空且不超过 128 个 Unicode 字符；最多 100 条引用，允许空集合表达没有原文引用。引用必须对应该资产的具体文案版本，quote 非空且为其精确子串；不得把乙视频的文案标成甲视频。文案 ID 自身固定版本，不接受“最新”占位。资产元数据和组织关系不作为文案引用。

| 工具 | 输入 | 输出与行为 |
| --- | --- | --- |
| `create_note` | `asset_id,body,source` | `{id,origin:"ai"}`；新增 general 笔记，模型与引用使用现有 Note 字段；不创建结构化拆解或比较记录 |
| `create_category` | `name,parent_id` | `{id}`；创建分类，沿用名称 / 父级与重名校验，不提供改名或删除 |
| `add_asset_categories` | `asset_id,ids` | `{asset_id,ids}`；向一个资产追加 1～100 个现有分类 ID，整个批次同事务 |
| `add_asset_tags` | `asset_id,ids` | `{asset_id,ids}`；向一个资产追加 1～100 个现有标签 ID，不创建标签 |
| `create_topic` | `input:{title,description},source` | `{id,origin:"ai"}`；创建独立研究专题，保存 AI 来源与文案引用；不追加或编辑已有成员 |
| `create_idea` | `input:{title,viewpoint,body,references},source` | `{id,origin:"ai"}`；创建独立选题草稿，最多 100 项 video / clip / topic 参考，沿用既有种类、归属与存在性校验 |

自己的观点 / 正文与参考内容继续分开。新建操作不接受已有记录 ID、修订或原文替换参数，不能覆盖人工笔记、整理稿、原文、专题或草稿。关系重复添加不产生重复行；无效成员使整个追加批次回滚。分类重名返回 `CATEGORY_EXISTS`。创建工具不具备请求去重，不应在超时或断连后自动重试；先通过界面或读取工具确认是否已提交。超时或客户端退出不等于撤销已经提交的短写入。

六工具均 `readOnlyHint=false`、`destructiveHint=false`、`openWorldHint=false`；仅两项追加关系 `idempotentHint=true`。注解供客户端使用，实际权限由 Core 执行。

## 来源与持久化

笔记沿用 `origin=ai`、model_name 和 citations。分类和分类 / 标签关系是组织信息，不包含生成正文的来源字段。专题与草稿增加只读 `ai_source:{model_name,citations}`；不存在表示原有人工创建，创建后人工编辑仍保留该来源。界面在详情展示“AI 创建”、模型名称与可展开的精确引用，不把历史来源改成“当前正文全部由 AI 生成”。

schema v11 为 research_topics / library_ideas 增加可空 ai_source_json；v1～v10 升级先做一致性备份，迁移事务失败不改变旧版本。writer v9 / reader v1～v9；旧包缺元数据按人工来源读取，v1～v8 夹带 AI 元数据拒绝。备份恢复校验模型与精确引用关系，完整保留来源；既有文案、片段、专题成员、选题参考和图片规则不变。

成功提交后发送对应 scope 的 `library_changed`，失败不通知。适配器只传明确 DTO，不直接访问数据库；原生路径仅用于桌面配置，不包含在工具结果。

## 错误与验证边界

公开错误只允许固定 code：`WRITE_DISABLED`、`INVALID_INPUT`、`INVALID_CITATION`、`ASSET_NOT_FOUND`、`CATEGORY_NOT_FOUND`、`CATEGORY_EXISTS`、`TAG_NOT_FOUND`、`TOPIC_INVALID`、`IDEA_INVALID`、`IDEA_REFERENCE_INVALID`、`IDEA_REFERENCE_EXISTS`、`SERVICE_UNAVAILABLE`、`CORE_WRITE_FAILED`。底层 SQL、路径或错误参数不透传。业务错误保持连接可读，Core / IPC 断连沿用进程回收规则。

Windows 验证见[第 28 批记录](workbench-knowledge-base-validation.md#第-28-批wb-310-受控-mcp-写入)，Apple Silicon 的原生授权、六工具及来源备份恢复验收见[第 31 批记录](workbench-knowledge-base-validation.md#第-31-批macos-原生组合验收与取消回收修复)；Intel 无可用机器，保持 UNAVAILABLE。真实模型自主生成写入结果的质量评价不等于协议验收，本批不调用模型账号。
