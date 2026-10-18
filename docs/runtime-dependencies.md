# 运行依赖

2026-10-18 用户授权。入口：**系统设置 → 运行依赖**。协议 v3（新增依赖命令），引擎 0.7.0；旧 0.6.0 sidecar 不具备此能力，必须与应用一起重建分发。

每次应用启动自动检查一次；前端应用层持有检查结果、错误与正在进行的依赖操作，切换主菜单或设置分类不重新发起检查。自动检查失败后保留错误，由用户点击“重新检测”重试；安装完成后使用后端返回的重新检测结果。仅保留本次运行状态，不将依赖可用状态持久化到下一次启动。

## 行为与边界

- 检测复用 `python-engine/local_tools.py`，实际执行版本命令。FFmpeg 必须可执行；YouTube 的 JavaScript 运行时要求 Deno 2.3+ 或 Node.js 22+，满足其一即可。无法执行、低版本或过期链接按不可用处理。
- 当前进程 PATH、注册表 PATH、常见工具安装目录的发现保持原样；系统候选后补充应用数据目录 `runtime/ffmpeg`、`runtime/javascript`。后续媒体操作使用同一目录和发现逻辑。
- 用户点击缺失项才下载：FFmpeg/FFprobe 6.1.1、Deno 2.9.7。Windows x64、macOS Intel、macOS Apple Silicon 各有固定下载产物；其他系统/架构可检测已有工具，但不猜测下载包。
- 使用固定 HTTPS 来源、固定大小与 SHA-256，逐跳限制 GitHub 及其 Release/源码域名；不提供镜像、远程脚本、shell 安装器、管理员提权或任意 URL。使用系统代理，不携带平台 Cookie/代理快照。
- 下载到 Rust 创建的专属暂存目录；真实字节进度经 JSONL/Tauri Channel 返回。每文件先检查大小与 SHA-256，再解压：gzip 有输出上限，zip 只读取固定可执行文件，不整体释放归档路径。
- 保留上游 LICENSE、FFmpeg 构建说明及 SOURCE.txt。实际运行版本通过后才提交专属目录，完成后重新检测。macOS 对二进制执行本地 ad-hoc 签名；不安装全局包、不改变 PATH、不覆盖系统工具。
- 依赖操作串行，复用 Rust Core worker 生命周期和 PythonBridge 进程组/Windows Job。取消或退出应用会终止并回收整棵进程树，再清理暂存目录；失败可重试。网络读取 15 秒、单次下载最多 300 秒、整次安装最多 600 秒；不提高媒体网络超时。
- 内置 Python、yt-dlp、DTK 不通过此功能更新，继续随软件发布。仅补充外部运行工具。

## 来源

交互流程参考 [Downvid runtime.ts](https://github.com/Downvid/downvid/blob/46a18d10649dc5329ed69fe502d5349703ca8c66/electron/utils/runtime.ts) 与依赖页；核对版本 commit `46a18d10649dc5329ed69fe502d5349703ca8c66`、MIT。未复制其代码或引入 Go sidecar、aria2、镜像及独立 yt-dlp 升级。

实际资产及许可见 [vendor/UPSTREAM.md](../vendor/UPSTREAM.md) 的“运行依赖下载”。当前版本/hash 由生产 `runtime_assets.py` 固定，升级需同步核对各架构产物、许可和执行验证。

## 验证

- Python：新依赖单元测试和既有 YouTube 测试，覆盖托管发现、Node 替代、hash/长度失败、版本失败、许可、重定向与隔离；离线测试不替代真实下载。
- Rust：依赖检查/安装进度、互斥、取消、退出回收与暂存清理；原桌面核心回归。
- Windows 在线：生产下载代码在隔离工作目录中模拟系统工具缺失，真实下载两类工具、核对 pinned SHA-256、解压、执行 FFmpeg/FFprobe/Deno 并经媒体使用的发现函数命中。用户系统安装和应用配置不改动。
- Windows PyInstaller：重建引擎并验证握手/依赖检测；浏览器 IPC fixture 与真实 Tauri 检测分开记录。
- macOS 自动下载/签名/执行：本机 Windows 无法实测，`NOT_RUN`。全新无 Python 系统验收仍属于 M3。
