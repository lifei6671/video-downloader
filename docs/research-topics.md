# 研究专题（WB-308）

## 使用与引用

从知识库“研究专题”进入，可新建专题、编辑标题和研究内容。一个专题组合多个视频、片段和笔记；资料仍归原资产，专题只保存引用、加入时名称和成员顺序，不复制正文或视频。比较笔记也是可加入的笔记，其内部固定来源规则继续有效。

资料选择器使用现有知识库搜索和分页，再选择视频、该视频的片段或笔记。相同专题内同种资料不能重复添加；不同专题可引用同一来源。成员支持移除和上移 / 下移，按钮可用键盘操作，首尾不能越界。移除仅删除成员关系，不删除资产、片段、笔记、图片或媒体文件。

成员列表读取当前来源，笔记展示当前修订而非加入时的旧正文。来源归档后仍显示“已移出知识库”并可打开；文件缺失或变化分别显示状态，片段状态复用 clips 领域的原始 Source 判断，重新关联新视频不使旧片段冒充可播放。来源记录删除后保留加入时名称、种类、资产 ID / 资料 ID 和顺序，标“引用缺失”，不能自动改绑其他同名资料，可继续移除或排序。

打开视频进入原资产详情，专题编辑区隐藏且不同时参与交互，返回恢复原专题。片段打开原范围播放器，笔记打开当前正文、创建来源和引用原文；比较笔记可继续查看其固定来源。对话框关闭释放视频，忽略迟到读取并恢复焦点。三个规定视口沿用既有布局 token，长内容在本区域滚动，长标题换行或截断保留 title。

专题正文显式保存，未保存内容沿用导航保护。成员变更前先保存正文，避免将成员顺序更新与未保存草稿混为一次提交。写入冲突保留当前草稿；资料选择器内显示失败原因。过期添加需要回读时只禁止再次写入，关闭仍可用，不能把“需回读”当成“正在请求”锁住弹窗。提交成功但回读失败明确提示已保存，禁止重复写入，重新读取后恢复当前关系。

## 数据与命令

schema **v9** 新增：

| 表 | 字段与边界 |
| --- | --- |
| `research_topics` | `id,title,description,revision,created_at,updated_at` |
| `research_topic_members` | `id,topic_id,kind,asset_id,resource_id,label,asset_title,position,created_at`；kind 仅 video / clip / note；同专题、种类、资料唯一 |

成员外键只指向专题，来源 ID 不设级联删除，确保引用缺失后仍可见。视频成员要求 `resource_id == asset_id`；新增片段 / 笔记须实际存在且属于所声明资产。名称快照至多 512 个 Unicode 标量，笔记标题取首个非空行至多 120 个；快照不包含源文件绝对路径或正文副本。读取失败返回错误，不冒充“引用缺失”。

| Tauri 命令 | 入参 / 返回 |
| --- | --- |
| `list_library_topics` | → `ResearchTopic[]` |
| `get_library_topic` | `topicId` → `{topic,members}`，成员附当前标题、归档 / 缺失、文件状态及笔记修订 |
| `create_library_topic` | `input:{title,description}` → ID |
| `edit_library_topic` | `topicId,input,expectedRevision` |
| `add_library_topic_member` | `topicId,input:{kind,asset_id,resource_id},expectedRevision` → member ID |
| `remove_library_topic_member` | `topicId,memberId,expectedRevision` |
| `reorder_library_topic_members` | `topicId,memberIds,expectedRevision`；完整成员 ID 全排列 |

修改、增删、排序使用专题级 CAS；关系与 revision / updated_at 在同一事务提交，失败整体回滚，移除后重新连续编号。空专题可以保存，但不允许覆盖已有专题恢复“空库”。写入成功发送 `library_changed` 的 topics scope；资产、笔记和片段变化也会刷新当前专题状态。这些命令仍属于桌面调用；WB-310 的 create_topic 单独映射相同创建领域服务，并保存 AI 来源 / 文案引用，见[受控写入](mcp-write-tools.md)。

## 备份与迁移

从 schema v1～v8 升级前执行一致性数据库备份，事务加入 v9 表；失败保留旧版本并可重试。WB-308 引入逻辑包 v7；当前 WB-310 writer **v9**，reader **v1～v9**，增加[选题与草稿](ideas-and-drafts.md)，新增 `research_topics` / `topic_members` 白名单集合，保留 ID、修订、名称快照和顺序。旧包缺字段视为空集合；旧版本包夹带专题数据拒绝。

恢复验证专题标题 / 内容 / 修订、成员唯一性、专题外键、连续顺序、来源种类与 ID / 资产一致性。允许合法缺失来源引用，现存片段 / 笔记必须仍属于原资产；归档由恢复后的资产决定，不持久化可用状态。含 / 不含媒体的两次备份恢复都保留成员关系，缺媒体仍可读笔记。只有空专题、没有资产的库也计为非空，备份范围和结果返回专题数量；恢复前后均拒绝覆盖非空知识库。

## 验证

Rust 专题测试覆盖三类成员、去重、错误归属、全排列 / CAS、原子失败、移除保护、归档 / 缺失 / 重关联状态、schema 升级回滚、一致性备份、空专题库及两轮含 / 不含媒体恢复和恶意关系。浏览器验证三视口、资料选择、顺序、视频返回、笔记 / 播放 / 键盘、失败和冲突恢复；Windows 原生验证真实 Tauri / SQLite / 媒体和空闲重开。Apple Silicon 已完成原生专题 / 成员 / 来源、三窗口与键盘、范围播放、重启和两种备份恢复；7 项专题核心回归通过。Intel 无可用机器，继续 UNAVAILABLE。实际结果见[第二十六批记录](workbench-knowledge-base-validation.md#第二十六批wb-308-研究专题)及[第 34 批记录](workbench-knowledge-base-validation.md#第-34-批macos-研究专题原生验收)。
