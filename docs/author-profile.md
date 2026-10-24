# 作者资料

2026-10-24 用户授权：抖音、TikTok、YouTube 展示作者 ID、昵称和头像；头像字节保存在 SQLite。Telegram 使用频道身份，不识别个人发帖者。

- `author_profile` 为可选对象：`id`、`nickname`、`handle`、`kind=user|channel`。ID 按字符串保存。旧记录仍可显示原有 `author`，不自动联网补全历史任务。
- 抖音沿用 DTK 的 `sec_uid` 身份语义，账号名另存 `handle`；TikTok 使用数字作者 ID；YouTube 使用 `UC…` 频道 ID，`@handle` 单独保存。昵称不再统一加 `@`。
- 抖音/TikTok 的 yt-dlp 适配从已有详情响应补充字段，DTK 视频和图集读取已有 Author；不切换引擎、不新增签名算法。
- YouTube 只根据单视频解析得到的频道 ID读取 `/channel/<id>/about` 的频道头像元数据，不枚举频道视频。复用锁定版本 yt-dlp 的页面/缩略图解析能力，并核对频道 ID；网络通过 GuardedRH、使用该平台配置。头像额外允许 `yt3.ggpht.com`、`yt3.googleusercontent.com`。
- 头像资料请求和头像下载各使用 5 秒网络超时，头像上限 2 MiB，只接受 JPEG/PNG/WebP。暂存名称固定 `author-avatar`，Rust 校验字节数及格式后写入 `parse_cache.author_avatar BLOB`；迁移仅添加可空列。远程头像 URL 不进入事件、任务快照或 SQLite。
- 头像是可选资源：资料缺失、网络失败或头像无效不影响作品解析/下载。头像通过 `get_author_avatar(taskId)` 读取任务对应缓存，前端使用 Blob URL 和 shadcn Avatar 占位。预览与任务列表展示昵称、ID和头像，搜索支持作者昵称/ID。
- 相同作品再次解析、作者 ID 不变时可保留已有头像；作者 ID 改变时清空旧头像。移除最后一个同作品任务或预览时，沿用封面引用清理事务清空头像，保留作者文字元数据与解析别名。
- Telegram 当前 tdl HTTP 接口不提供频道标题/头像。展示已确认的频道 ID、输入链接中可用的公开频道用户名；缺少公开名时显示“Telegram 频道”。不将频道 ID 冒充个人 ID，不推测标题或头像。

协议仍为 v3，Python/Rust 引擎版本同步到 0.13.0，需要重新构建 sidecar；可复用解析缓存按引擎版本失效。

上游依据：[yt-dlp TikTok/Douyin](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/tiktok.py)、[YouTube 频道解析](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/youtube/_tab.py)。不修改 vendored 上游。

点击列表昵称进入本地搜索结果页，显示该作者已有的作品记录（包括尚未提交下载的解析预览）。有 ID 时按平台和作者 ID 精确匹配；旧记录只有作者字符串时按相同平台的账号名精确回退。已知 ID 不同的记录不会因昵称或账号名相同而混入。只有昵称且没有可匹配身份的资料保留普通文字。Telegram 按频道 ID 聚合。搜索框显示作者昵称，手动编辑后恢复普通记录搜索；侧栏返回下载任务时退出作者筛选。作者结果页不显示批量清理历史按钮，避免将作者筛选误当删除范围。

列表清晰度按真实宽高的短边显示 `1080P`、`720P` 等标签，悬停保留完整尺寸；未知尺寸显示未知。下载时间使用实际完成时间，格式为 `2026年10月16日 20:48`，未知显示 `—`。本地文件缺失提醒和清晰度、时间、大小同处一行，红字无边框，窄窗口允许自然换行。
