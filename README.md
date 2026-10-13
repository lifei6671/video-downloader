# video-downloader

抖音 / TikTok 单视频桌面下载器，当前进入 M2 任务与历史管理阶段，采用 Tauri/React 与 shadcn/ui 亮色界面。后端已统一使用 yt-dlp 解析和下载，Rust 管理操作进程、取消与文件提交；旧引擎及生产依赖已移除。

**双平台实际下载已通过**：抖音直连，TikTok 单独使用代理；源码短链与 Windows 打包长链均通过完整解码。抖音详情请求使用经用户批准的最小签名补丁。全新无 Python Windows 环境仍待验证，用户批准移至 M3 打包阶段。详见 [M1 验证记录](docs/m1-validation.md)、[技术方案](docs/tauri-desktop-technical-plan.md)与[依赖来源](vendor/UPSTREAM.md)。

## Windows 开发与测试

需要 Rust/Cargo、uv；在仓库根目录 PowerShell 执行：

```powershell
uv venv --python 3.12 .venv
uv pip sync --python .venv/Scripts/python.exe python-engine/requirements-dev.txt
cargo build --manifest-path src-tauri/Cargo.toml --bin m1-probe
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
.venv/Scripts/python.exe -m pytest -q
```

pytest 包含 Rust/Python 进程集成测试，必须先构建探针；测试使用隔离 HTTP fixture，不访问真实平台。运行依赖固定 yt-dlp 2026.8.19，协议 v2，具体版本见锁文件。

## 真实下载验证

TikTok 便捷入口默认读取 `tests/TikTok.txt`、Edge 150 UA 和 SOCKS5 `127.0.0.1:3067`：

```powershell
.venv/Scripts/python.exe scripts/run_m1.py
.venv/Scripts/python.exe scripts/run_m1.py --proxy http://127.0.0.1:3067
.venv/Scripts/python.exe scripts/run_m1.py --direct
```

可用 `--url`、`--cookie-file`、`--user-agent`、`--output-dir` 覆盖 TikTok 样本。双平台或抖音验证使用配置文件。将以下内容保存到忽略目录 `.local/probe.json`，替换有效作品链接；抖音使用直连，TikTok 单独配置代理。UA 应与 Cookie 来源一致，但不代表 TLS 浏览器指纹相同。

```json
{
  "inputs": ["https://www.douyin.com/video/作品ID", "https://www.tiktok.com/@作者/video/作品ID"],
  "profiles": {
    "douyin": {"cookie_file": "tests/douyin.txt", "proxy": null},
    "tiktok": {"cookie_file": "tests/TikTok.txt", "proxy": "http://127.0.0.1:3067"}
  },
  "output_dir": ".local/downloads"
}
```

```powershell
.venv/Scripts/python.exe scripts/run_m1.py .local/probe.json
```

默认 UA 为 Edge 150；可在各 profile 中设置 `user_agent`。`proxy: null` 明确直连，不继承环境代理。没有 Cookie 时省略 `cookie_file`。相对路径按执行时工作目录解释；Cookie 文件是单行 `name=value; name=value`，仅在内存读取，经 stdin 传递，不进入命令参数或日志。`tests/douyin.txt`、`tests/TikTok.txt` 已忽略，其他私有文件放在 `.local/` 或仓库外，不得提交。

探针串行处理输入，每次解析和下载分别启动一个 Python 操作进程。下载重新解析规范链接，核对作品身份后写入任务暂存目录；Rust 验证并提交 `平台_作品ID.mp4/webm`，已有同名文件不覆盖，重复验证使用新输出目录。仅支持直接 HTTP(S) 单文件视频，不支持图集、直播、分轨、HLS/DASH、转码或断点续传。

`resolved` 表示解析成功，`progress` 表示字节进度，`completed` 表示该文件提交成功。任何输入失败时整体退出码为 1，其他成功文件保留；配置错误为 2。Ctrl+C 取消网络操作、回收进程并清理暂存。独立调用 bridge 时须持续保持 stdin 打开直到进程退出；EOF 会终止操作。M1 不负责异常崩溃后的暂存恢复，桌面任务恢复见 M2 验证记录。

## Windows 引擎打包

```powershell
.venv/Scripts/python.exe -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec
.venv/Scripts/python.exe scripts/run_m1.py .local/probe.json --probe src-tauri/target/debug/m1-probe.exe --engine python-engine/dist/video-engine/video-engine.exe
```

必须一起分发 `video-engine/` 整个目录，包含 `_internal` 与许可证。运行器仅供开发便利；原生探针可直接启动打包 exe，通过 stdin 接收 JSON（调用方将 `cookie_file` 转换为内存 `cookie`）。当前已验证隔离 PATH 与 cwd 的原生调用，尚未在全新无 Python 虚拟机验证，也不是 Tauri 安装包；macOS 需单独构建与实测。

[旧引擎 M1 记录](docs/m1-legacy-validation.md)和[独立 yt-dlp 试验记录](docs/ytdlp-validation.md)仅供历史参考，旧脚本与引擎已退役。

## M2 桌面开发

先按上文构建 PyInstaller 引擎，然后执行：

```powershell
npm ci
npm run desktop
```

前端检查：`npm run build`。完整桌面编译：`npm run desktop:build`（不生成安装器）。Rust 桌面门禁：`cargo test --manifest-path src-tauri/Cargo.toml`、`cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`。

在设置页分别保存两平台 Cookie、User-Agent 与 HTTP/SOCKS5 代理，再粘贴作品链接。Cookie 直接存本机应用数据目录的 SQLite，在设置页多行文本框回显；含密码的代理存系统凭据。下载目录默认为系统下载目录下的 Yingqu，可通过系统目录选择器修改。解析最多同时一个，下载默认同时 2 个、可在设置调整为 1–3 个，超出名额按提交顺序等待；提高上限立即补位，降低不打断已有下载。取消等待进程退出与暂存清理；重启保留记录，未完成任务标记中断，不自动恢复下载。

并发队列、手动重试、移除历史及二次确认删除本地视频已实现。重试新建任务、重新解析并从头下载；解析失败且无作品信息时需重新粘贴链接。Debug 开发可执行文件依赖仓库内的打包引擎目录；Release 从应用资源目录加载完整引擎。Windows 干净环境与 macOS 验收在 M3 完成。[M2 单任务验证](docs/m2-validation.md) · [并发与队列验证](docs/m2-queue-validation.md) · [重试与历史管理验证](docs/m2-history-validation.md)。

新下载同时保存平台可用的最高分辨率封面，文件名为视频同名加 `.cover.jpg/png/webp`，已完成列表读取本地封面并支持重启。封面获取失败不把已成功视频误报为失败；旧历史不自动联网补封面。删除本地视频保留封面供历史展示，从列表移除不删除视频或封面文件。

## 界面构建版本

Logo 区域和关于页版本号在 Vite 构建时注入，优先级为 `APP_VERSION` → GitHub Actions 标签事件的 `GITHUB_REF_NAME`（`GITHUB_REF_TYPE=tag`）→ `package.json` 的 version。标签如 `v1.2.3` 显示为 `1.2.3`，支持预发布版本。普通分支构建不显示分支名。

标签发布 workflow 在临时 CI checkout 中将 `package.json`、npm 锁文件、Cargo 清单/锁文件及 Tauri 配置同步到标签版本，并设置 `APP_VERSION`；不会提交这些版本变更。已构建产物不会随 Release 名称变化。

应用内容区禁用右键菜单，Logo 区域为静态展示，无按钮悬停效果。

## GitHub Actions 标签发布

[Release 工作流](.github/workflows/release.yml) 监听 `v*` 标签推送，并严格校验正式版本 `vMAJOR.MINOR.PATCH`（例如 `v0.1.0`；暂不接受预发布或任意名称标签）。代码和工作流需要先提交到被打标签的 commit，然后推送标签：

```powershell
git tag v0.1.0
git push origin v0.1.0
```

工作流分别在 Windows x64、macOS Intel、macOS Apple Silicon 原生 runner 上安装 Node 24、Python 3.12 和 Rust stable，执行锁文件安装、离线测试、PyInstaller onedir 打包、隔离 cwd/PATH 的引擎启动检查，以及 Tauri 前端/桌面安装器构建。安装包包含整个 `video-engine/` 目录；Release 使用应用资源路径，不依赖构建机源码位置。

三个构建全部成功后，使用仓库自带 `GITHUB_TOKEN` 创建草稿、上传 Windows NSIS `.exe` 和两种 macOS `.dmg`，随后公开 Release。构建失败不发布；已有同名 Release 不覆盖。若上传中断留下草稿，先检查并删除该草稿，再重跑失败的发布 job。Actions artifacts 保留 14 天。仓库/组织策略须允许该 workflow 的 `contents: write`，无需新增发布 Token。

当前未配置 Windows 代码签名或 Apple Developer ID/公证；macOS 仅 ad-hoc 签名。自动构建和离线启动检查不代表安装验收：全新无 Python Windows、两种 macOS 安装/启动、双平台在线下载与签名公证仍待验证。首次标签 CI 运行结果需单独确认。

## 持续集成

[CI 工作流](.github/workflows/ci.yml) 在推送到 `master` 时自动触发，在 Windows x64、macOS Intel / Apple Silicon 上运行离线 Python/Rust 测试、Rust Clippy、打包引擎启动检查，以及前端类型检查和桌面 Release 编译（不生成安装器、不发布 Release）。同一分支的新推送会取消仍在运行的旧 CI。CI 不使用真实 Cookie 或执行平台在线下载；实际 GitHub runner 结果以 Actions 为准。
