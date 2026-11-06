# CSP 组件审计与备份设置通知验证

本次检查基于当前 Windows Tauri、React、Radix 与锁文件依赖。保留原 CSP，不启用任意内联脚本或样式；不修改云端空间、凭据和媒体。

## 确认的问题与实现

| 路径 | 修复前证据 | 当前处理 |
| --- | --- | --- |
| Sonner | 原生动态 STYLE 存在但没有生效的 sheet，Toast 进入普通文档流 | 沿用上一轮正式 CSS 导入，运行时无 nonce 的冗余 STYLE 仍会被拒绝，但通知不依赖它 |
| Radix Select.Viewport | 打开下拉框后，动态隐藏滚动条的 STYLE 被拦，scrollbar-width 为 auto | 传入 Tauri 为启动样式生成的当前文档 nonce |
| react-remove-scroll / react-style-singleton | 下拉框打开后 body 已有 data-scroll-locked，但 overflow 仍为 visible，滚动锁 STYLE 被拦 | 应用入口初始化 get-nonce，让原有库自行生成带 nonce 的测量与锁定规则 |
| 远端备份表单 | 原生捕获 TypeError: Cannot read properties of undefined (reading 'protocol')，调用栈来自空间 Select.onValueChange | 忽略加载期间空值、当前值及不在选项内的空间 ID；提供商仅接受 r2/s3 |

后一个错误来自 Radix 在表单内同步受控选择值时发出的空事件，不是云端请求失败。原生逐项读取 get_backup_settings、list_backup_spaces、get_backup_credentials、list_backup_assets、list_backup_operations 均成功，诊断只记录成功状态，没有输出凭据值。不会用隐藏 Toast 掩盖真实失败，凭据读取失败仍显示原有 CREDENTIAL_ERROR 文案。

`get-nonce` 使用锁文件原有的 1.0.1，仅补充直接依赖声明，没有引入另一份版本。nonce 来自 Tauri 已授权的启动 STYLE，不是应用自造或持久化值。[Tauri CSP 说明](https://v2.tauri.app/security/csp/)描述其构建与运行时的 nonce/hash 机制。

## 审计范围与回归

| 组件/能力 | 代码检查 | 浏览器生产构建与 CSP 验证 |
| --- | --- | --- |
| Select | 动态 viewport STYLE 与滚动锁 | 三视口 scrollbar-width:none、body overflow:hidden、Esc 关闭 |
| Dialog、AlertDialog、Sheet | 共用 react-remove-scroll | 三视口定位、滚动锁、关闭释放与焦点恢复 |
| DropdownMenu、ContextMenu | 共用 react-remove-scroll / Popper | 三视口定位、滚动锁、Esc 关闭、指针事件恢复 |
| Tooltip | Popper 使用 CSSOM style 属性，未发现动态 STYLE 依赖 | 三视口显示及窗口边界 |
| SVG 图标与圆形进度 | Lucide 使用内联 SVG，Progress 使用 CSS 锥形渐变，未发现动态 STYLE 注入 | SVG 图标尺寸、stroke，圆形进度渐变与 42% 数值 |
| Accordion、Checkbox | 已打包 CSS 与 React style 属性 | 展开收起及勾选 |
| 其他现有基础控件、Markdown、动画与字体 | 样式来自现有外部 CSS/字体或 CSSOM 属性，无额外运行时 STYLE 生成点 | 现有启动、备份、布局回归通过 |

依赖源码中另有 Radix ScrollArea 的动态 STYLE，但当前应用没有 ScrollArea 组件或调用，不把未使用代码当作已验收能力。

实际命令与结果：

- `npm run build`：PASS，类型检查与正式前端资源生成成功，保留既有构建警告。
- `npm ls get-nonce --all`：PASS，直接与传递依赖均复用 1.0.1。
- 独立组件 fixture 生产构建：PASS，命令见 [开发文档](development.md#通知样式与原生-csp)，不进入正常桌面构建入口。
- `tests/csp-components-browser.mjs`：PASS，1440×900、1280×800、1024×680；每视口 5 类浮层、Select、Tooltip、SVG；备份表单 9 次卸载/重挂载没有错误 Toast 或误切空间；主动切空间与真实凭据错误提示通过；0 pageerror。测试同时确认无 nonce 的任意 STYLE 仍被拒绝。
- `tests/settings-layout-browser.mjs`：PASS，三视口，逐帧 47/46/47；保留 CSP 拒绝运行时通知样式的边界。
- `tests/multi-device-sync-browser.mjs`：PASS，三视口、6 次空间切换、冲突/导出/懒加载/Telegram/升级目标检查。
- `tests/startup-paint-browser.mjs`：PASS，三视口、启动交接和延迟更新说明。
- `git diff --check`：PASS。

## 原生验证边界

`npm run desktop:build`：PASS，包含打包引擎预检、TypeScript/Vite 和正常 Windows Release 构建，Rust 编译 31.08s，输出 `src-tauri/target/release/yingqu.exe`，没有生成另一文件名的修复版。

实际启动新 `yingqu.exe` 验证：远端备份保持选中，从关于、任务、下载页面各返回两轮，共 6 次，标题 top=0、错误 Toast 数量=0。打开备份空间下拉框后，原生 body overflow=hidden、viewport scrollbar-width=none，滚动锁及 Select 的两份动态 STYLE 均带当前文档 nonce，sheet 有效；Esc 关闭后释放滚动锁。只检查 UI 和成功状态，没有写入用户配置或打印凭据值。

浏览器 fixture 的所有浮层结果不等同于每一种原生浮层都逐项点击过；本次原生直接验证的是 Select 和共享滚动锁路径。macOS Intel、macOS Apple Silicon：NOT_RUN。未执行真实云端写入、升级、传输、删除或安装包验收。
