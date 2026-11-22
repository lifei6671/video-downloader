# 平台登录与下载设备信息

内置登录适用于抖音、小红书、TikTok、B 站和腾讯元宝。登录窗口保持系统 WebView 的原生 User-Agent、navigator 和 Client Hints，不设置自定义 UA，不注入伪造的平台或硬件信息。

## 登录环境与会话

Windows/Linux 按平台在应用本地数据目录的 `platform-browser/<platform>` 保存浏览器环境。macOS 14+ 使用按平台固定的 WebKit Data Store ID；macOS 13.3 使用 WebKit 原生持久存储，按站点来源隔离数据。任何系统均不对共享浏览器存储执行全局清空。

每次用户主动登录前，只移除该平台的认证标记 Cookie，保留设备 Cookie 和 LocalStorage，避免旧认证 Cookie 被直接认作新登录。关闭或取消窗口不覆盖应用中已保存的会话。成功后同时保存 Cookie 和当时读取的浏览器身份，使用同一 SQLite 事务；保存失败时保留原值。

`platform_settings.login_device` 是可空 JSON，包含真实 UA、浏览器提供的 Client Hints，以及平台、语言、时区、屏幕尺寸、可获取的逻辑处理器数量和浏览器公开的内存档位。浏览器未提供的字段不补造。Cookie 沿用现有存储契约，不进入普通设置摘要、事件、参数日志或命令行。

“清除会话”移除所选平台的应用会话、设备绑定和站点 Cookie，不清空其他平台，也不全局删除 LocalStorage。重新登录使用同一浏览器存储。旧会话没有设备绑定，仍保留原行为；界面提示重新登录以绑定当前设备。

## 请求使用规则

- 内置登录生效时，Rust 任务配置快照使用会话绑定的 UA、Client Hints 和浏览器环境；后续手动 UA 编辑不改变内置会话。切换到手动 Cookie 时使用其独立 UA，不带入内置登录的 Client Hints。
- Client Hints 只发往对应平台的第一方站点/API。每次请求和重定向都重新限制范围，媒体、头像等 CDN 不继承这些请求头。认证 Cookie 仍由现有作用域 CookieJar 管理。
- 抖音 DTK 详情和作者扫描使用绑定的浏览器平台、屏幕、语言、时区等信息。浏览器不提供或上游不支持的字段沿用固定 DTK 的默认处理，不修改签名算法。
- YouTube 仍在用户的外部浏览器登录；手动 Cookie 必须使用来源浏览器的 UA，不读取或猜测外部浏览器的身份，不自动改成应用 WebView 的信息。
- Telegram 使用现有 SDK 默认检测到的系统、版本和设备位数，保留应用 API 身份与应用版本，不再把设备型号写死为应用名称。

已排队任务保持捕获的配置。确需刷新登录凭据的既有路径同时刷新配套 UA 和设备信息。数据库备份包含 Cookie 所对应的身份快照，但不包含浏览器目录；换设备后应重新登录以绑定新环境。

这只统一应用可复用的设备信息，不让 Python HTTP 客户端变成浏览器，也不保证平台不再要求验证码、扫码确认或其他验证。

## 验证

- Rust 覆盖五个平台绑定、认证切换、配置编辑、重启、事务失败回滚、请求快照与 Cookie 清除范围；桌面构建验证 WebView API。
- Python 覆盖 Client Hints 第一方边界、重定向清理、协议校验及 DTK 使用实际浏览器环境。Node 覆盖 Chromium、WebKit 和高熵字段被拒绝的采集结果。
- 浏览器使用模拟 IPC，在三个设计视口检查五个平台的绑定状态、手动 UA 编辑、会话清除和 YouTube 外部登录，并使用当前 Chromium 验证采集脚本不改写 navigator。
- 真实账号重新登录、平台设备列表识别与风控改善、macOS 原生登录验收尚未执行，不能由离线测试或浏览器模拟替代。

原生 API 依据：[WebView2 User-Agent](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings2)、[User-Agent Client Hints](https://learn.microsoft.com/en-us/microsoft-edge/web-platform/how-to-detect-win11)。存储和 JS 回调使用锁定 Tauri/Wry 的 `data_directory`、`data_store_identifier` 和 `eval_with_callback`。
