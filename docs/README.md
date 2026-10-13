# 技术文档

- [开发、验证与发布](development.md)：源码运行、测试、引擎打包、CI 与标签发布。

- [技术方案](tauri-desktop-technical-plan.md)：yt-dlp 架构、协议、阶段与验收标准。
- [设置与封面验收](m2-cover-settings-validation.md)：Cookie 多行回显、等高设置卡、并发下拉与本地封面。
- [M2 重试与历史管理验收](m2-history-validation.md)：新任务重试、仅移除记录、二次确认删除文件。
- [M2 并发与队列验收](m2-queue-validation.md)：默认 2、可设 1–3、FIFO 调度与取消隔离。
- [M2 第一张卡验收](m2-validation.md)：shadcn/ui 亮色桌面、SQLite、双平台真实单任务闭环。
- [当前 M1 验证](m1-validation.md)：迁移、测试、双平台真实结果及未通过项。
- [旧 M1 历史记录](m1-legacy-validation.md)：旧引擎结果，不计入当前验收。
- [独立 yt-dlp 试验记录](ytdlp-validation.md)：迁移前的 TikTok 下载证据。

用户已批准进入 M2，已完成单任务闭环、并发等待队列及重试/历史管理；全新无 Python Windows 验收移至 M3、仍待验证。启动和构建见[开发文档](development.md)。
