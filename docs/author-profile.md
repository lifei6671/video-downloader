# 作者资料

抖音、TikTok、YouTube、小红书和 Telegram 的头像字节保存在 SQLite。Telegram 使用频道身份和频道头像。

- `author_profile` 为可选对象：`id`、`nickname`、`handle`、`kind=user|channel`。ID 按字符串保存。旧记录仍可显示原有 `author`，不自动联网补全历史任务。
- 抖音沿用 DTK 的 `sec_uid` 身份语义，账号名另存 `handle`；TikTok 使用数字作者 ID；YouTube 使用 `UC…` 频道 ID，`@handle` 单独保存。昵称不再统一加 `@`。
- 抖音/TikTok 的 yt-dlp 适配从已有详情响应补充字段，DTK 视频和图集读取已有 Author；不切换引擎、不新增签名算法。
- 小红书从笔记详情的 `user` 对象读取 `userId` 和 `avatar`。网页分享链接和 App 短链共用视频及图集的头像缓存流程。
- YouTube 只根据单视频解析得到的频道 ID读取 `/channel/<id>/about` 的频道头像元数据，不枚举频道视频。复用锁定版本 yt-dlp 的页面/缩略图解析能力，并核对频道 ID；网络通过 GuardedRH、使用该平台配置。头像额外允许 `yt3.ggpht.com`、`yt3.googleusercontent.com`。
- Python 引擎的头像资料请求和下载各使用 5 秒网络超时，并使用固定暂存名 `author-avatar`。Telegram SDK 读取小头像的整个阶段限时 5 秒，直接返回图片字节。头像上限 2 MiB，只接受 JPEG、PNG 和 WebP。Rust 校验后写入 `parse_cache.author_avatar BLOB`。远程头像 URL 不进入事件、任务快照或 SQLite。
- 头像是可选资源。资料缺失、网络失败或头像无效时，作品仍可解析和下载。作品预览、下载任务和任务详情通过 `get_author_avatar(taskId)` 读取缓存字节，在署名前显示头像，缺失时显示占位。可用作者昵称或 ID 搜索本地记录。
- 相同作品再次解析、作者 ID 不变时可保留已有头像；作者 ID 改变时清空旧头像。移除最后一个同作品任务或预览时，沿用封面引用清理事务清空头像，保留作者文字元数据与解析别名。
- Telegram 原生 SDK 从已解析的频道资料读取真实标题，写入 `author_profile.nickname` 和兼容的 `author`。频道 ID 用于本地作者搜索，完整 ID 在任务详情可查。历史记录缺标题时沿用已有名称或公开用户名，否则显示“Telegram 频道”。解析公开或数字频道链接时，使用同一频道的头像标识读取小头像。下载使用已有头像缓存，历史任务不会自动联网补全。下载或重试可更新已确认的频道名。频道改名后，媒体文档身份和原文件名仍按已有记录识别。

作者字段沿用 JSON Lines v3 的可选资料契约；Telegram SDK 解析缓存使用独立修订，不改变 Python 引擎协议或备份协议。

上游依据：[yt-dlp TikTok/Douyin](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/tiktok.py)、[YouTube 频道解析](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/youtube/_tab.py)。不修改 vendored 上游。

点击列表昵称进入本地搜索结果页，显示该作者已有的作品记录（包括尚未提交下载的解析预览）。有 ID 时按平台和作者 ID 精确匹配；旧记录只有作者字符串时按相同平台的账号名精确回退。已知 ID 不同的记录不会因昵称或账号名相同而混入。只有昵称且没有可匹配身份的资料保留普通文字。Telegram 按频道 ID 聚合。搜索框显示作者昵称，手动编辑后恢复普通记录搜索；侧栏返回下载任务时退出作者筛选。作者结果页不显示批量清理历史按钮，避免将作者筛选误当删除范围。

列表清晰度按真实宽高的短边显示 `1080P`、`720P` 等标签，悬停保留完整尺寸；未知尺寸显示未知。下载时间使用实际完成时间，格式为 `2026年10月16日 20:48`，未知显示 `—`。本地文件缺失提醒和清晰度、时间、大小同处一行，红字无边框，窄窗口允许自然换行。
