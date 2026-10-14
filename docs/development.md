# 开发、验证与发布

本文面向开发者，以下命令均在仓库根目录执行。产品介绍与使用流程见[项目首页](../README.md)。

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

pytest 包含 Rust/Python 进程集成测试，必须先构建探针；测试使用隔离 HTTP fixture，不访问真实平台。运行依赖固定 yt-dlp 2026.8.19 与 DTK 5.1.3 白名单切片，协议 v3，具体版本见锁文件。

## macOS 开发与打包

需要 Xcode Command Line Tools、Rust/Cargo、Node/npm 和 uv；Intel 与 Apple Silicon 必须分别在对应架构的 Mac 上构建 Python 引擎，不混用 Windows 产物或另一架构的 `.venv`。以下命令在仓库根目录的 zsh 执行：

```sh
uv venv --python 3.12 .venv
uv pip sync --python .venv/bin/python python-engine/requirements-dev.txt
uv pip check --python .venv/bin/python
npm ci
cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe
cargo test --locked --manifest-path src-tauri/Cargo.toml
.venv/bin/python -m pytest -q
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings
.venv/bin/python -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec
.venv/bin/python scripts/check_packaged_engine.py python-engine/dist/video-engine/video-engine
npm run desktop
```

生成包含完整引擎的应用与磁盘映像：

```sh
APPLE_SIGNING_IDENTITY=- npm run tauri -- build --features desktop --config src-tauri/tauri.release.conf.json --bundles app,dmg -- --locked
codesign --verify --deep --strict src-tauri/target/release/bundle/macos/映取.app
.venv/bin/python scripts/check_packaged_engine.py src-tauri/target/release/bundle/macos/映取.app/Contents/Resources/video-engine/video-engine
open src-tauri/target/release/bundle/macos/映取.app
```

应用位于 `src-tauri/target/release/bundle/macos/映取.app`，DMG 位于相邻的 `bundle/dmg/`。`-` 是本地 ad-hoc 签名；按 2026-10-14 用户决定，macOS 不要求 Developer ID、证书或公证，不将这些设置为本次验收阻塞项。本地签名检查不代表下载后的 Gatekeeper 场景已验证。

当前 Tauri CLI 按 Cargo 的 `[[bin]]` 声明顺序签名，因此 `m1-probe` 必须排在主程序 `yingqu` 前，避免主程序签名检查同包中尚未签名的辅助程序。日常 CI 的两种 macOS 架构均生成 ad-hoc 签名的 `.app`，并执行 `codesign --verify --deep --strict`；Release 另生成 DMG。

`tauri.macos.conf.json` 自动合并，声明最低 macOS 13.3，以匹配现有 Tailwind 4 的 Safari/WebKit 16.4 要求；13.3 实机兼容性仍待验证。依据：[Tailwind 浏览器要求](https://tailwindcss.com/docs/compatibility)、[WebKit 16.4](https://webkit.org/blog/13966/webkit-features-in-safari-16-4/)。Apple Silicon 本机证据及剩余验收见 [macOS 验证记录](macos-validation.md)。

开发依赖锁文件包含平台标记：`macholib` 只在 macOS 安装，`pefile` / `pywin32-ctypes` 只在 Windows 安装。更新锁文件使用 `uv pip compile --universal python-engine/requirements-dev.in -o python-engine/requirements-dev.txt`，避免再次生成只适用当前主机的清单。

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

探针串行处理输入，每次解析和下载分别启动一个 Python 操作进程。下载重新解析规范链接，核对作品身份后写入任务暂存目录；Rust 验证并提交 `平台_作品ID.mp4/webm`，已有同名文件不覆盖，重复验证使用新输出目录。支持直接 HTTP(S) 单文件视频及图集；图集保存为任务独占目录，按实际格式编号，不转码。直播、分轨、HLS/DASH、转码或断点续传不支持。

`resolved` 表示解析成功，`progress` 表示字节进度，`completed` 表示该文件提交成功。任何输入失败时整体退出码为 1，其他成功文件保留；配置错误为 2。Ctrl+C 取消网络操作、回收进程并清理暂存。独立调用 bridge 时须持续保持 stdin 打开直到进程退出；EOF 会终止操作。M1 不负责异常崩溃后的暂存恢复，桌面任务恢复见 M2 验证记录。

## Windows 引擎打包

```powershell
.venv/Scripts/python.exe -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec
.venv/Scripts/python.exe scripts/run_m1.py .local/probe.json --probe src-tauri/target/debug/m1-probe.exe --engine python-engine/dist/video-engine/video-engine.exe
```

必须一起分发 `video-engine/` 整个目录，包含 `_internal` 与许可证。运行器仅供开发便利；原生探针可直接启动打包 exe，通过 stdin 接收 JSON（调用方将 `cookie_file` 转换为内存 `cookie`）。当前已验证隔离 PATH 与 cwd 的原生调用，尚未在全新无 Python 虚拟机验证，也不是 Tauri 安装包；macOS 需单独构建与实测。

[旧引擎 M1 记录](m1-legacy-validation.md)和[独立 yt-dlp 试验记录](ytdlp-validation.md)仅供历史参考，旧脚本与引擎已退役。

## M2 桌面开发

先按上文构建 PyInstaller 引擎，然后执行：

```powershell
npm ci
npm run desktop
```

前端检查：`npm run build`。完整桌面编译：`npm run desktop:build`（不生成安装器）。Rust 桌面门禁：`cargo test --manifest-path src-tauri/Cargo.toml`、`cargo clippy --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`。

在设置页分别保存两平台 Cookie、User-Agent 与 HTTP/SOCKS5 代理，再粘贴作品链接。Cookie 直接存本机应用数据目录的 SQLite，在设置页多行文本框回显；含密码的代理存系统凭据。下载目录默认为系统下载目录下的 Yingqu，可通过系统目录选择器修改。解析最多同时一个，下载默认同时 2 个、可在设置调整为 1–3 个，超出名额按提交顺序等待；提高上限立即补位，降低不打断已有下载。取消等待进程退出与暂存清理；重启保留记录及已解析预览，解析中/排队/下载中标记中断，不自动恢复下载。可复用缓存和封面/图集字节保存在 SQLite，命中时不请求详情，下载时仍重新解析；见 [解析缓存](parse-cache-validation.md)。

并发队列、手动重试、移除历史及二次确认删除本地作品已实现。重试新建任务、重新解析并从头下载；解析失败且无作品信息时需重新粘贴链接。Debug 开发可执行文件依赖仓库内的打包引擎目录；Release 从应用资源目录加载完整引擎。Windows 干净环境与 macOS 验收在 M3 完成。[M2 单任务验证](m2-validation.md) · [并发与队列验证](m2-queue-validation.md) · [重试与历史管理验证](m2-history-validation.md)。

新下载同时保存平台可用的最高分辨率封面，文件名为视频同名加 `.cover.jpg/png/webp`，已完成列表读取本地封面并支持重启。封面获取失败不把已成功视频误报为失败；旧历史不自动联网补封面。删除本地视频保留封面供历史展示，从列表移除不删除视频或封面文件。

## 界面构建版本

Logo 区域和关于页版本号在 Vite 构建时注入，优先级为 `APP_VERSION` → GitHub Actions 标签事件的 `GITHUB_REF_NAME`（`GITHUB_REF_TYPE=tag`）→ `package.json` 的 version。标签如 `v1.2.3` 显示为 `1.2.3`，支持预发布版本。普通分支构建不显示分支名。

标签发布 workflow 在临时 CI checkout 中将 `package.json`、npm 锁文件、Cargo 清单/锁文件及 Tauri 配置同步到标签版本，并设置 `APP_VERSION`；不会提交这些版本变更。已构建产物不会随 Release 名称变化。

应用内容区禁用右键菜单，Logo 区域为静态展示，无按钮悬停效果。

## GitHub Actions 标签发布

[Release 工作流](../.github/workflows/release.yml) 监听 `v*` 标签推送，并严格校验正式版本 `vMAJOR.MINOR.PATCH`（例如 `v0.1.0`；暂不接受预发布或任意名称标签）。代码和工作流需要先提交到被打标签的 commit，然后推送标签：

```powershell
git tag v0.1.0
git push origin v0.1.0
```

工作流分别在 Windows x64、macOS Intel、macOS Apple Silicon 原生 runner 上安装 Node 24、Python 3.12 和 Rust stable，执行锁文件安装、离线测试、PyInstaller onedir 打包、隔离 cwd/PATH 的引擎启动检查，以及 Tauri 前端/桌面安装器构建。安装包包含整个 `video-engine/` 目录；Release 使用应用资源路径，不依赖构建机源码位置。

三个构建全部成功后，使用仓库自带 `GITHUB_TOKEN` 创建草稿、上传 Windows NSIS `.exe` 和两种 macOS `.dmg`，随后公开 Release。构建失败不发布；已有同名 Release 不覆盖。若上传中断留下草稿，先检查并删除该草稿，再重跑失败的发布 job。Actions artifacts 保留 14 天。仓库/组织策略须允许该 workflow 的 `contents: write`，无需新增发布 Token。

当前未配置 Windows 代码签名；macOS 按用户决定采用 ad-hoc，不要求 Developer ID / 证书或公证。自动构建和离线启动检查不代表安装验收：全新无 Python Windows、Intel macOS 与旧系统仍待验证；Apple Silicon 本机安装启动及双平台 GUI 下载 / 重启证据见 [macOS 验证记录](macos-validation.md)。首次标签 CI 运行结果需单独确认。

## 持续集成

[CI 工作流](../.github/workflows/ci.yml) 在推送到 `master` 时自动触发，在 Windows x64、macOS Intel / Apple Silicon 上运行离线 Python/Rust 测试、Rust Clippy、打包引擎启动检查，以及前端类型检查和桌面 Release 编译（不生成安装器、不发布 Release）。同一分支的新推送会取消仍在运行的旧 CI。CI 不使用真实 Cookie 或执行平台在线下载；实际 GitHub runner 结果以 Actions 为准。

## 系统托盘与单实例

关闭主窗口会隐藏到系统托盘，已开始的解析和下载继续运行。左键点击托盘图标或选择“显示窗口”可恢复主窗口；“退出映取”先停止队列、取消并等待引擎进程回收，再退出。重复启动应用会显示、还原并聚焦已有窗口，不再打开第二个应用实例。macOS 点击 Dock 图标也会恢复窗口；这里的单实例指桌面应用，WebView 和下载引擎仍可拥有自己的辅助进程。

## 解析模块同步与补丁升级

应用标识为 `me.disign.yingqu`。首次启动在新数据目录尚不存在时，将原 `com.yingqu.desktop` 数据目录迁移到新目录，保留 Cookie、设置、历史和提交意图；已有新目录时不合并或覆盖。系统代理密码沿用原 Keychain/Credential Manager 服务命名空间，保证原凭据引用继续可用。macOS 数据库为 `~/Library/Application Support/me.disign.yingqu/yingqu.sqlite`。

固定版本离线校验及重建（PowerShell 使用 `.venv/Scripts/python.exe`）：

```sh
.venv/bin/python scripts/update_dtk_vendor.py --check
.venv/bin/python scripts/update_dtk_vendor.py
# 可改为读取已有上游 checkout 的 git blobs，不改变该 checkout：
.venv/bin/python scripts/update_dtk_vendor.py --source /path/to/upstream
```

审核新的完整提交 SHA 后同步：

```sh
.venv/bin/python scripts/update_dtk_vendor.py --ref <完整40位SHA> --update
```

同步脚本只读取 manifest 的白名单，不自动增加服务文件。先检查当前 vendor 未被手改，再从原始 git 内容重建 staging、校验/应用独立补丁、审计导入和语法；失败保留已安装源码与 manifest。版本号、原始及安装后文件 hash、补丁 hash、生成 revision 一起更新。

`vendor/dtk/` 的源码及补丁通过 `.gitattributes` 固定为 LF，避免 Windows checkout 的 CRLF 转换改变校验字节；检查器使用 UTF-8 和正斜杠清单路径。已有工作区若仍为 CRLF，应先确认没有本地修改，再重新检出该目录，不要重算 manifest 哈希来接受换行差异。

补丁冲突时先在临时上游副本检查包初始化变化，修改 `vendor/dtk/patches/desktop-imports.patch`；禁止修改 `upstream/` 已安装源码。签名和 parser 算法不改。审核补丁后使用相同 `--ref <SHA> --update` 重新生成，并检查 diff。若导入闭包增加，应明确审核 manifest 白名单、运行依赖及许可证；脚本不会自动引入未审核模块。依赖变化时生成两个跨平台锁文件并同步环境：

```sh
uv pip compile --universal python-engine/requirements.in -o python-engine/requirements.txt
uv pip compile --universal python-engine/requirements-dev.in -o python-engine/requirements-dev.txt
uv pip sync --python .venv/bin/python python-engine/requirements-dev.txt
uv pip check --python .venv/bin/python
.venv/bin/python scripts/update_dtk_vendor.py --check
cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe
.venv/bin/python -m pytest -q
cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings
npm run build
```

握手核对 manifest 的 DTK commit/version。源码变更后必须重新生成 sidecar，再构建完整应用。离线升级回归包括新版本同步、补丁冲突、未知服务依赖、本地修改保护、原图优先及 404/410 降级、网络/身份错误不降级、多文件校验和恢复。离线通过后，用有效视频与图集分别执行双平台真实冒烟，并在 Windows、macOS arm64、macOS Intel 各自验收安装包；不得用手工 fixture 或旧引擎结果替代。此次结果见 [双引擎验收](dual-engine-validation.md)。
