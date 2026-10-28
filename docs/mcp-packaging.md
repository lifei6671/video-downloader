# MCP 适配进程分发与安装验证

对应 [WB-207](workbench-knowledge-base-tasks.md)。已接入构建、许可和安装包流程，并完成 Windows x64 NSIS 的实际安装、配置复制、启动、连接、重启及卸载验证。WB-208 已补充 [Windows 真实客户端与模型组合验收](mcp-client-acceptance.md)。macOS Intel / Apple Silicon 继续待验收；未发布版本或修改日常客户端配置。

## 构建与目录契约

生产构建显式启用 `desktop,mcp`。复用已有 Cargo `[[bin]]`，Tauri 收集启用 feature 的二进制，不增加另一套 externalBin 复制或按需下载机制。依据当前锁定 CLI 2.11.4 的[二进制收集实现](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-cli/src/interface/rust.rs)；`yingqu-mcp` 的 `required-features=["mcp"]` 必须在打包时显式满足。

| 平台 | 主程序旁的适配程序 | 许可 |
| --- | --- | --- |
| Windows NSIS 安装目录 | `yingqu-mcp.exe`，与主程序同目录 | `licenses/mcp-licenses.txt` |
| macOS `.app`（待验收） | `Contents/MacOS/yingqu-mcp`，与 CFBundleExecutable 同目录 | `Contents/Resources/licenses/mcp-licenses.txt` |

设置页继续使用当前运行主程序的位置生成配置，客户端通过独立 command / args 启动这个原生程序。适配程序没有 Node.js、Python 或 Cargo 运行时要求；这些只用于开发或测试客户端。主应用原有下载引擎继续按原契约完整打包，没有将 MCP 改为 Python 服务。

`package.json` 的 `desktop:build`、日常 CI 与 Release 构建入口均包含 `mcp`。CI / Release 的 Windows 分支生成 NSIS 后实际安装到仓库 `.local` 中的新目录，校验后卸载；macOS 分支检查 `.app` 中的二进制与许可，并保留 deep / strict 签名检查。当前 Windows 不签名、macOS ad-hoc 签名的既有决策不变；Cargo 的辅助程序先于主程序声明，避免包内签名顺序变化。未修改更新下载、校验和启动安装器的机制，不另建适配程序更新器；辅助程序由同一安装包交付。

## 完整许可

`vendor/mcp-licenses.txt` 保留锁定依赖的完整上游许可文件，不只写 SPDX 名称或链接：rmcp 3.8.0、rmcp-macros 3.8.0、futures 0.3.34、futures-executor 0.3.34、pastey 0.2.3、schemars_derive 1.2.3、serde_derive_internals 0.30.0，以及已有 URI 直接依赖 percent-encoding 2.3.2。共 8 包、13 份原文，含宏依赖、双许可全文和 SDK 许可过渡说明。

`scripts/update_mcp_licenses.py` 从锁定 Cargo cache 生成，比较 `desktop,mcp` 与 `desktop` 的当前宿主依赖图，新增包未覆盖或已审版本变化会失败。`--check` 不写文件；CI 先构建以填充宿主依赖 cache，再在安装检查和产物上传前比对已审文件，防止全新 Release runner 尚未下载可选桌面依赖而使离线 metadata 失败。升级 SDK / 锁文件时应重新审查清单和许可，不能直接忽略检查。完整文本作为 release resources 安装，检查器逐字节比对安装文本与源文件。

## 安装检查器

`scripts/check_windows_installer.ps1` 接受明确安装器与受限 RunId，在 `.local/wb207-install/<RunId>/安装 Program Files` 新建安装；目标目录已存在时拒绝覆盖。NSIS `/D` 为最后参数，路径本身不加 shell 引号。成功或失败均按本次目标清理，只有显式 `-KeepInstalled` 且所有检查成功时保留用于原生 UI 验收。脚本记录安装器 / 安装后程序 SHA-256，不读取日常应用数据。

`scripts/check_packaged_mcp.py` 验证主程序、适配程序与完整许可，再以仅系统 PATH、隔离 home 和临时工作目录启动 helper。无参数必须返回退出码 2、stdout 为空和固定 Usage，证明已安装的原生程序能启动；实际握手 / 领域读取由 SDK 和客户端验证完成，不能用 Usage 检查替代连接验收。

Windows 还对比安装内容与构建输出：helper 必须全字节一致；主程序仅允许 Tauri 将首个 `__TAURI_BUNDLE_TYPE_VAR_UNK` 改为 NSIS 的 `NSS` 标记。Tauri 在生成安装包后恢复构建目录的原主程序，因此不能直接要求两者 SHA 相同。此规则依据[固定版本 bundler 的打补丁与恢复流程](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle.rs)，其他字节差异仍拒绝，并有回归测试。

## Windows 本轮证据

使用正常 release 构建与资源配置，额外合并本地验收配置：`identifier=me.disign.yingqu.wb207`、`productName=Yingqu WB207`、`mainBinaryName=yingqu-wb207`。仅在 `.local` 保存覆盖配置，不修改生产标识；独立产品名、进程名和预建的空数据目录标记避免与日常程序 / 卸载项 / 迁移混用。

实际安装路径为 `.local/wb207-install/wb207-final/安装 Program Files`。应用从内嵌 `http://tauri.localhost/` 启动，不使用 Vite。设置页默认关闭，开启后复制的 TOML / JSON 路径均指向安装目录；Python tomllib 解析等价，官方 TypeScript SDK 在系统 PATH 与隔离 home 下启动该 helper，发现七工具并读取分类，Codex CLI 0.162.1 使用原样 TOML 发现七工具。关闭后旧配置不能连接，重新打开应用默认关闭，开启后旧配置可以重连。未发起模型 turn。

安装目录中的现有下载引擎也通过离线启动检查。SDK / Codex 结束后 helper 进程为 0；界面关闭服务后再清理隔离主程序，卸载实际删除主程序和 helper。另以 CI 默认不保留安装的路径执行完整脚本通过。证据见[第十七批](workbench-knowledge-base-validation.md#第十七批wb-207-mcp-安装包与-windows-安装验收)。

WB-207 本批未运行模型；后续 WB-208 的 Windows 组合证据见 [真实客户端验收](mcp-client-acceptance.md)。macOS 两种架构、全新无开发环境设备、跨版本在线自动更新和 Windows SmartScreen / 企业部署仍不计为通过。构建日志仍有既有 use client、chunk-size 和 STATIC_VCRUNTIME 废弃提示，不改变既有签名策略来消除这些提示。
