# Streamable HTTP MCP 验证

日期：2026-10-29，目标分支 master。按用户要求，当前对外 MCP 接入改为仅本机 Streamable HTTP，移除命令启动与独立 helper 分发；未提交、推送或发布。

## 实现与接口

| 源码 | 职责 |
| --- | --- |
| `src-tauri/src/mcp_http.rs` / `mcp_http_tests.rs` | 官方 SDK HTTP 监听、Host/Origin、请求大小、取消与有界关闭、专项回归 |
| `mcp_adapter.rs` / `mcp_adapter/ipc.rs` / `resources.rs` | 保留工具、媒体内容块、资源和 Prompts；每请求独立内部 IPC 连接 |
| `mcp_ipc_api.rs` / `desktop_core.rs` | 单一 Core 的 HTTP/IPC 生命周期、关闭清权、媒体回收和退出 |
| `mcp_connection.rs` / `desktop.rs` | 仅返回实际 HTTP 地址；复制地址；移除旧复制配置和公开 IPC 启停命令 |
| `src/components/mcp-settings.tsx` / `src/lib/mcp-api.ts` | 开关、读写授权、地址显示与复制，无命令或格式选择 |
| Cargo、打包检查器、许可证脚本 | desktop 包含 mcp；不生成 yingqu-mcp binary；主程序与 21 个 MCP 依赖完整许可核验 |
| SDK、原生和浏览器检查脚本 | 全部当前接入改为 HTTP URL；保留隔离 Core 与本机原生测试资料库 |

固定地址为 `http://127.0.0.1:17653/mcp`。仅绑定 IPv4 loopback；端口占用明确失败并回滚内部 IPC。SDK 只允许实际 Host authority 及同源 Origin，拒绝外站和 null Origin。HTTP 请求体上限 64 KiB，超限为 413；内部 IPC 响应 1 MiB 和普通/音频期限保留。

桌面 `get_mcp_connection` / `set_mcp_service_enabled` / `set_mcp_write_enabled` 返回 `{status:{enabled,endpoint,error},write_enabled}`；新增 `copy_mcp_connection_url()`。不再返回 adapter_path/configs，不再注册旧 copy_mcp_client_config、get_mcp_ipc_status、set_mcp_ipc_enabled。原内部 IPC 及资料服务保留。SQLite schema11、backup9 未变，既有 Tauri 版本未升级。

## 自动验证

| 命令 | 结果 |
| --- | --- |
| `npm run build` | PASSED |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib -- --test-threads=1` | 278 passed，2 ignored（既有在线测试） |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --all-targets -- -D warnings` | PASSED |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --features desktop,mcp --lib mcp_http -- --test-threads=1` | 3 passed |
| `python -m pytest -q tests/test_mcp_packaging.py` | 8 passed |
| `python scripts/update_mcp_licenses.py --check` | PASSED，21 packages |
| `node --test tests/material-model-checks.test.mjs` | 7 passed |
| `tests/mcp-settings.browser.js`、`tests/mcp-writing.browser.js` | 各 4 组通过，1440×900 / 1280×800 / 1024×680、键盘、复制、写权限及异步失败恢复 |

地址验证器另验证 6 组非法/旧参数在文件读取前失败，文档中的 `--unavailable` 调用通过；它只验证官方 SDK 连接，不声明运行 Codex。证据为 `config-args-check.log`。

HTTP 专项包含真实 FFmpeg 引擎/子进程及临时资源回收、HTTP 断连、服务关闭、慢请求体、读写权限变化、端口占用回滚、Host/Origin、超量请求。测试只使用临时 SQLite 和合成媒体。

`node scripts/verify-mcp-adapter.mjs` 的 `--library` / `--writes` / `--resources` / `--media` / `--operations` / `--scope` / `--av` 均通过 HTTP。包含 2025-11-25 SDK 握手、2026-07-28 discovery 与 metadata、PNG、可解码 WAV、收录范围、来源绑定、进程/暂存回收、转录操作 ID、关闭地址失效及重新开启恢复只读。

阶段证据目录：`.local/mcp-http-followup/`。官方 SDK 结果包括 `.local/wb204/run-X3jX7i`、`.local/wb310/run-y11DNZ`、`.local/wb205/run-rYEnZ7`、`.local/wb312/run-0PDWfM`、`.local/wb311/run-pztzzX`、`.local/wb204/run-tQTXdk`、`.local/materials-av/run-dU932s`。

## 真实客户端与模型

使用已授权的本机 Codex 登录，通过官方原生 `codex-cli 0.165.0-alpha.1.2`、gpt-6.1-sol 和 URL 配置连接本轮隔离合成 Core；未复制凭据、未修改用户客户端配置、未访问日常素材库。HTTP 证据为 `.local/materials-av/run-jMLVEg` 和 `.local/materials-codex/client-oXMAo1`。

- 文本读取和模型分析 PASSED。
- 五帧实际 PNG 内容块读取与模型图像分析 PASSED，保留对应素材/文件/帧及时间引用。
- SDK 真实 WAV 接收与解码 PASSED；Codex 协议层收到 text/audio。
- 当前模型输入记录为 text/text，模型明确报告不支持实际音频输入，`actualModelAudioConsumption=UNAVAILABLE`。不声称模型听到了声音；这一客户端能力限制不因换 HTTP 改变。
- 客户端自然退出，exit 0，无强制清理，未启用继承的其他 MCP 服务。

## Windows 原生与平台边界

Windows 使用独立 identifier `me.disign.yingqu.p1a-mcp-http-20261029-upgrade`，通过现有 p1a 脚本重编并启动原生 Tauri 窗口。前端来自 Vite，Core、系统剪贴板、监听和权限为真实原生命令，不是浏览器 mock，也不是安装包。

原生验证 PASSED：默认关闭；关闭时拒绝复制；开启后只读地址、系统剪贴板与实际 `127.0.0.1:17653` listener 一致；官方 SDK 按复制地址连接；只读拒绝写入；界面开启写授权后可创建分类；撤销后仍可读且写入被拒绝；关闭服务后 SDK 不能连接且端口无监听。证据为 native-settings.json、native-url-sdk.log、native-readonly.log、native-write.log、native-write-revoked.log、native-service-off.log。测试后已关闭 MCP 和写权限，窗口保留供查看。

macOS Intel / Apple Silicon 当前 HTTP 原生验证 UNAVAILABLE，不能由历史 stdio/MCP/ASR 记录替代。安装包本轮 NOT_RUN；打包回归仅验证主程序/许可与既有签名失败传播，不宣称新安装包已验收。完整原生视听分析、真实平台下载与模型质量不是本轮 HTTP 传输验证结果。

旧 workbench/topic/idea 默认界面已退役，其数据和内部兼容测试保留；当前权限浏览器脚本只验证仍存在的权限界面。历史专用 profile 原生写入/转录脚本已适配 URL，未在本轮重新运行其旧专题/选题流程。
