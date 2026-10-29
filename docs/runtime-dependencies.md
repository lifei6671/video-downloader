# 运行依赖

2026-10-18 用户授权工具下载，2026-10-23 将语音模型更换为 Fun-ASR-Nano。入口：**系统设置 → 运行依赖**。协议 v3，当前引擎 0.12.0；旧 sidecar 不具备完整能力，必须与应用一起重建分发。

每次应用启动自动检查一次；前端应用层持有检查结果、错误与正在进行的依赖操作，切换主菜单或设置分类不重新发起检查。自动检查失败后保留错误，由用户点击“重新检测”重试；安装完成后使用后端返回的重新检测结果。仅保留本次运行状态，不将依赖可用状态持久化到下一次启动。

## 行为与边界

依赖各自使用独立卡片和操作：缺失 FFmpeg 的卡片提供“下载 FFmpeg”，JavaScript 卡片提供“下载 Deno”，模型卡片提供“下载语音模型”；点击只安装该项，进度和取消留在同一卡片，不提供汇总下载按钮。下载、修复和取消按钮与依赖说明共用卡片内容区，不设置独立灰色操作页脚或分隔线。依赖安装仍串行，安装期间其他下载按钮禁用。已可用系统工具不重复安装，模型保留显式修复入口；tdl 沿用自行安装与路径选择。运行依赖标题、依赖说明和“重新检测”组成固定头部，只有下方卡片列表滚动。卡片滚动容器保留 1px 边框留白、右侧 12px 内容间距，保持 shadcn Card 的外侧 ring 四边完整显示、卡片不贴住滚动条。

- 检测复用 `python-engine/local_tools.py`，实际执行版本命令。FFmpeg 必须可执行；YouTube 的 JavaScript 运行时要求 Deno 2.3+ 或 Node.js 22+，满足其一即可。无法执行、低版本或过期链接按不可用处理。
- 当前进程 PATH、注册表 PATH、常见工具安装目录的发现保持原样；系统候选后补充应用数据目录 `runtime/ffmpeg`、`runtime/javascript`。后续媒体操作使用同一目录和发现逻辑。
- 用户点击缺失项才下载：FFmpeg/FFprobe 固定使用上游 `b6.1.1` Release 资产，Windows x64 / macOS Intel 二进制实际版本为 6.1.1，macOS Apple Silicon 为 6.0.0；按平台校验实际版本。Deno 为 2.9.7。其他系统/架构可检测已有工具，但不猜测下载包。
- 使用固定 HTTPS 来源、固定大小与 SHA-256，逐跳限制 GitHub 及其 Release/源码域名；不提供镜像、远程脚本、shell 安装器、管理员提权或任意 URL。使用系统代理，不携带平台 Cookie/代理快照。
- 下载到 Rust 创建的专属暂存目录；真实字节进度经 JSONL/Tauri Channel 返回。每文件先检查大小与 SHA-256，再解压：gzip 有输出上限，zip 只读取固定可执行文件，不整体释放归档路径。
- 保留上游 LICENSE、FFmpeg 构建说明及 SOURCE.txt。实际运行版本通过后才提交专属目录，完成后重新检测。macOS 对二进制执行本地 ad-hoc 签名；不安装全局包、不改变 PATH、不覆盖系统工具。
- 依赖操作串行，复用 Rust Core worker 生命周期和 PythonBridge 进程组/Windows Job。取消或退出应用会终止并回收整棵进程树，再清理暂存目录；失败可重试。网络读取 15 秒、单次下载最多 300 秒、整次安装最多 600 秒；不提高媒体网络超时。
- 内置 Python、yt-dlp、DTK 和 CPU ASR 程序继续随软件发布。按需下载仅提供列出的工具和模型，不替换内置 CPU 程序。

## Fun-ASR-Nano 模型

模型卡片在说明旁持续展示“模型大小”，下载前、下载中和已安装状态均保留。数值取自所选版本的 `variant_model_bytes`，兼容未提供该字段时的 `download_bytes`（包含 VAD），按 GiB 四舍五入至一位小数；未返回有效大小时显示“未知”。

运行依赖页原有 FFmpeg、JavaScript、tdl 顺序保留，最后显示一张语音模型卡。`tool=fun-asr-nano` 对应固定 F16 编码器、Q8_0 解码器与 FSMN-VAD GGUF，合计 1275804800 bytes（约 1.2 GiB）；生产 URL、revision、大小、SHA-256 见 `python-engine/runtime_assets.py` 和[集成方案](fun-asr-nano.md)。Windows x64 同卡提供 CPU/CUDA 选择，macOS 保留 CPU。CUDA 模型约 1.0 GiB，首次另需程序和 CUDA DLL；详细切换、复用、显卡适配与回滚见 [CUDA 集成](cuda-acceleration.md)。不安装 PyTorch/FunASR Python 包。

CPU 模型安装在 `<应用数据目录>/runtime/fun-asr-nano/`，CUDA 在 `runtime/fun-asr-nano-cuda/`，仍为一项依赖。检测核对 manifest、文件存在性及大小；模型首次下载或“重新下载模型”修复时流式校验完整 SHA-256，再用内置短语音执行真实离线加载/推理，写入 manifest、SOURCE.txt、Apache-2.0 LICENSE 后整体提交。下载/校验/冒烟失败保留原安装，不将半成品标为可用；同尺寸损坏可以通过重新下载修复。

模型请求只允许固定 HTTPS 主机 `huggingface.co` 和本次实际重定向的 `us.aws.cdn.hf.co`；GitHub 工具下载边界保持独立。读取超时 15 秒，单文件下载最多 30 分钟，Rust 整次模型安装最多 45 分钟，冒烟期间持续心跳。FFmpeg/Deno 原有超时不变。取消、退出与暂存恢复复用同一 Core/PythonBridge 生命周期；模型安装与转录互斥，防止处理中替换模型。

下载模型不会自动提取视频文案，仍须用户点击“开始提取”。已有完整模型可离线识别；视频音频提取依赖现有 FFmpeg，缺失时单独点击下载。模型真实下载、中文路径和 Windows 推理验证见[Nano 集成与验证](fun-asr-nano.md)。

## 来源

交互流程参考 [Downvid runtime.ts](https://github.com/Downvid/downvid/blob/46a18d10649dc5329ed69fe502d5349703ca8c66/electron/utils/runtime.ts) 与依赖页；核对版本 commit `46a18d10649dc5329ed69fe502d5349703ca8c66`、MIT。未复制其代码或引入 Go sidecar、aria2、镜像及独立 yt-dlp 升级。

实际资产及许可见 [vendor/UPSTREAM.md](../vendor/UPSTREAM.md) 的“运行依赖下载”。当前版本/hash 由生产 `runtime_assets.py` 固定，升级需同步核对各架构产物、许可和执行验证。

## 验证

- Python：新依赖单元测试和既有 YouTube 测试，覆盖托管发现、Node 替代、hash/长度失败、版本失败、许可、重定向与隔离；离线测试不替代真实下载。
- Rust：依赖检查/安装进度、互斥、取消、退出回收与暂存清理；原桌面核心回归。
- Windows 在线：生产下载代码在隔离工作目录中模拟系统工具缺失，真实下载两类工具、核对 pinned SHA-256、解压、执行 FFmpeg/FFprobe/Deno 并经媒体使用的发现函数命中。用户系统安装和应用配置不改动。
- Windows PyInstaller：重建引擎并验证握手/依赖检测；浏览器 IPC fixture 与真实 Tauri 检测分开记录。
- macOS 自动下载/签名/执行：本机 Windows 无法实测，`NOT_RUN`。全新无 Python 系统验收仍属于 M3。

2026-10-23 UI 回归：`tests/runtime-dependencies.browser.js` 在 1440×900、1280×800、1024×680 检查固定标题/说明/重新检测、卡片与滚动条的 12px 间距、四张独立卡片的 ring 留白、内容完整性和键盘逐项下载；headed 模式另实际点击原生上下箭头验证双向滚动；同时覆盖进度归属、串行禁用、模型修复取消与不支持安装，共 5 组场景通过。既有 `tests/tdl-picker.browser.js` 的 11 组场景通过。前端类型/生产构建通过；fixture 不执行真实依赖下载。

## 旧安装保留

原 `runtime/sensevoice/`、`runtime/sensevoice-cuda/` 与 `sensevoice-backend.json` 保留，不自动删除，也不参与新模型可用性判断或后端选择。旧模型不能满足 Nano 依赖，用户需单独下载 Nano；旧文案可继续查看、复制和导出，新提取使用成功安装并选定的 Nano CPU 或 Windows CUDA 版。旧 SenseVoice CUDA 验证见[历史记录](sensevoice-transcription-validation.md)。
