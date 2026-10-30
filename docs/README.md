# 技术文档

- [开发、验证与发布](development.md)：源码运行、测试、引擎打包、CI 与标签发布。
- [视频 S3 备份与按需恢复方案](s3-video-backup-plan.md)：Cloudflare R2、本地优先备份、跨设备元数据恢复、视频懒加载与删除联动设计草案。
- [S3 备份实施任务](s3-backup-implementation.md)：M1–M6 数据、恢复与删除契约。
- [S3 备份验收记录](s3-backup-validation.md)：离线、浏览器、原生与真实 R2 分开记录。
- [S3 备份运维与接管](s3-backup-operations.md)：凭据、删除恢复、手动主设备迁移与专用测试。
- [单视频文案提取](fun-asr-nano.md)：Fun-ASR-Nano CPU / Windows CUDA、模型按需下载、旧结果保留与导出契约。
- [文案提取验证](nano-cuda-validation.md)：Nano Windows 真实模型、Tauri、回归与跨平台验收边界。
- [macOS 验证记录](macos-validation.md)：Apple Silicon 安装包、双平台 GUI 下载及正常重启恢复，以及 Intel / 旧系统验收边界。
- [解析缓存验收](parse-cache-validation.md)：SQLite 元数据/图片缓存、链接别名、重启预览及下载重新解析。
- [双引擎与图集验收](dual-engine-validation.md)：正式 macOS GUI、文件校验、重启及平台验收边界。
- [DTK 双解析引擎方案与试验](dtk-dual-engine-plan.md)：正式底层切片、导入补丁、图集任务契约和上游升级流程。

- [技术方案](tauri-desktop-technical-plan.md)：双引擎架构、协议、阶段与验收标准。
- [设置与封面验收](m2-cover-settings-validation.md)：Cookie 多行回显、等高设置卡、并发下拉与本地封面。
- [M2 重试与历史管理验收](m2-history-validation.md)：新任务重试、仅移除记录、二次确认删除文件。
- [M2 并发与队列验收](m2-queue-validation.md)：默认 2、可设 1–3、FIFO 调度与取消隔离。
- [M2 第一张卡验收](m2-validation.md)：shadcn/ui 亮色桌面、SQLite、双平台真实单任务闭环。
- [当前 M1 验证](m1-validation.md)：迁移、测试、双平台真实结果及未通过项。
- [旧 M1 历史记录](m1-legacy-validation.md)：旧引擎结果，不计入当前验收。
- [独立 yt-dlp 试验记录](ytdlp-validation.md)：迁移前的 TikTok 下载证据。

用户已批准进入 M2，已完成单任务闭环、并发等待队列及重试/历史管理；全新无 Python Windows 验收移至 M3、仍待验证。启动和构建见[开发文档](development.md)。

- [视频规格选择与抖音 4K 验收](video-format-validation.md)
- [TikTok 视频规格选择在线验收](tiktok-video-format-validation.md)

- [Fun-ASR-Nano Windows CUDA](cuda-acceleration.md)：独立 VAD、版本选择与下载边界；[本轮验证](nano-cuda-validation.md)。
