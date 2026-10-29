# MCP HTTP 服务分发与安装验证

MCP HTTP 服务集成在桌面进程中。`desktop` feature 包含 `mcp`，不再生成或打包 `yingqu-mcp` 独立命令程序；没有运行时额外下载或 helper 路径检查。

| 平台 | 程序 | 完整许可 |
| --- | --- | --- |
| Windows NSIS | 映取桌面主程序 | `licenses/mcp-licenses.txt` |
| macOS app | `Contents/MacOS` 中的主程序 | `Contents/Resources/licenses/mcp-licenses.txt` |

`vendor/mcp-licenses.txt` 由 `scripts/update_mcp_licenses.py` 从已锁定的 Cargo 包生成，包含 rmcp、HTTP 传输和媒体预览的完整许可文本。新增或变更依赖时更新明确版本清单并核验 `--check`。本轮复用官方 rmcp 3.8.0 和 axum 0.8，不升级既有 Tauri 依赖。

许可文件覆盖三个平台的固定包集合。脚本先审核当前平台新增依赖；ARM 图中缺少的 x86 专属包从 Windows 锁定依赖元数据补齐，不省略其许可。macOS 在运行离线生成器前执行 `cargo fetch --locked --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc` 缓存对应源码；不需要安装 Windows 编译工具链或构建 Windows 程序。CI 与 Release 已包含该准备步骤。

`scripts/check_packaged_mcp.py` 核验桌面主程序和完整许可，不启动额外命令程序。Windows 安装字节比对仅允许 Tauri 将首个 bundle marker 改为 NSIS 标记；其余差异仍拒绝。`scripts/check_windows_installer.ps1` 继续使用带 RunId 的隔离目录，保留已有安装、签名失败传播及清理边界。

安装文件完整性检查只证明文件与构建一致。实际 HTTP 启停、地址复制、SDK 连接和读写权限必须在独立原生资料库验证，不能用文件检查替代。历史 stdio helper 安装证据不用于确认当前 HTTP 构建；各平台当前结果见[HTTP MCP 验证](mcp-http-validation.md)。
