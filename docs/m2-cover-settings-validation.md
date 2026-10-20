# 设置与本地封面验收

> 2026-10-20 存储更新：新下载视频封面统一只存 SQLite 缓存，不再生成同目录封面文件；删除最后引用时清空 BLOB，旧版文件保持原样。下文同名封面文件及当时在线结果是旧版历史证据，当前契约见 [开发说明](development.md)。

当前存储回归：`cargo test --locked --manifest-path src-tauri/Cargo.toml cover -- --test-threads=1`（13 项）、`desktop_core::tests`（31 项）、`files::tests`（4 项）通过。覆盖三平台真实 sidecar 协议 fixture、封面字节入库、下载目录仅视频、重启/提交恢复、删除视频后保留、最后引用清理与事务回滚、缓存更新失败仍完成；图集回归保持。`tests/telegram-cover-cache.browser.js` 覆盖四平台完成事件及恢复记录的 8 组数据库封面展示，并检查三个设计视口。`cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` 通过。以上为隔离离线/浏览器 IPC 验证，不代表真实平台下载或原生 GUI 验收。

日期：2026-10-13。以本轮用户要求覆盖此前 Cookie 不回显、封面不落盘约定。未改变视频引擎、签名补丁、队列与取消契约。

## 交付

- 两平台设置卡并排等高，底部保存按钮对齐；长 Cookie 使用固定初始高度的多行输入，支持滚动、显示/隐藏、编辑、保存和再次进入时回显。
- 仅 get_profile_cookie 专用命令读取对应平台 Cookie；普通 get_settings、任务事件和日志仍不含 Cookie，代理密码仍不回显。
- 并发改用官方 shadcn/ui Select，1–3，默认 2，保存规则保持不变。
- 封面按已知像素数选择，尺寸缺失时优先 origin_cover/originCover，再按上游排序；使用同一 GuardedRH 网络校验及代理，限制 10 MiB、JPEG/PNG/WebP。补充实测 douyinpic.com 抖音域并测试假后缀及平台隔离。
- 视频旁保存同名 .cover.jpg/png/webp；Rust 校验并不覆盖提交，记录本地路径，列表通过任务 ID 读取成 Blob。重启可用，卸载释放 Blob URL。封面失败不使视频失败，显示封面未保存。
- 删除视频保留封面供历史展示；移除记录不删除视频或封面。旧完成记录不自动补封面。
- 首页预览维持源图比例、自适应宽高并约束最大尺寸；任务错误改为 icon + 小字，无边框。

## 验证

- PASS：21 Rust 单元/进程集成测试，覆盖封面提交、格式/大小/路径拒绝、不覆盖、重启读取、删除视频后保留封面、Cookie 专用读取与平台隔离。
- PASS：Rust Clippy（desktop / all-targets / -D warnings）、桌面 build、npm run build、PyInstaller 引擎重建。
- PASS：85 项 Python 全量与新增封面测试；既有网络/bridge/下载回归与封面失败、超限、优先原始图覆盖。
- PASS：真实 TikTok 下载 687985 字节视频与 42545 字节 JPEG，封面 960×540；真实抖音下载 19889228 字节视频与 25018 字节 JPEG，封面 480×360。按源图保存，不把低分辨率图放大冒称高清。
- PASS：应用重启后本地封面显示；首页 480×360 源图显示为 373×280，未强制固定 9:16。
- PASS：1440×900、1280×800、1024×680；并排卡片等高，长 Cookie 高度受限且无水平溢出；Select 保存并恢复原设置，任务错误无边框且含图标。测试后重启恢复原生窗口视口。
- PASS：真实 Cookie 回显内容与已存储值一致，仅记录布尔结果；截图遮挡输入内容，不保存凭据值。
- NOT_RUN：全新无 Python Windows、macOS Intel/Apple Silicon、安装包仍属 M3。

本机证据：`.local/cover-first-results.json`（首轮抖音图片域被拒，TikTok成功），`.local/cover-native-results.json`（抖音修复后），`.local/cover-layout-results.json`、`.local/cover-restart-results.json`及对应遮挡凭据的截图。最初长 Cookie 自动撑高和图片域拒绝均已修复并重验。
