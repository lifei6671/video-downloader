# 浅色桌面下载器 UI 调整汇总

2026-10-24。本记录汇总最终交付状态；此前各轮验证文档保留当时的尺寸与入口描述，当前规格以 `DESIGN.md` 和代码为准。

## 最终界面与交互

- 统一 Radix 浅色设计令牌、固定侧栏、白色工作区和共享弹窗样式，保留四个页面与原生窗口行为。
- 下载任务采用连续列表，完成行约 80px；状态与平台筛选可组合搜索，真实计数随任务事件更新。
- 行间无分隔线，悬停与选中使用圆角背景；播放仅显示图标。整行右键打开操作菜单，保留键盘、焦点恢复、菜单条件和删除语义。
- 菜单宽 236px，80ms 淡入、60ms 淡出，无位移；保留封面独立右键菜单。平台标签无下划线，空列表无外边框或“去下载作品”按钮。
- 空分享链接提交使用顶部 Toast 并聚焦输入框，重复提交不堆叠提示。
- 封面与图集读取使用共享缓存，跨页面复用 Blob URL并合并正在进行的读取。来源变化刷新，普通进度修订不刷新；未使用缓存受 128 张、64 MiB 限制，移除任务后释放。
- 任务缩略图居中等比例铺满，允许裁切边缘；首页和大图预览保留完整画面。原生滚动条按钮用渐变绘制箭头，上下按钮已实测。

## 验证证据

| 范围 | 实际结果 |
| --- | --- |
| TypeScript / Vite | `npm run build` PASS；保留既有依赖提示及大 chunk 警告 |
| 差异格式 | `git diff --check` PASS |
| 三个浏览器视口 | 1440×900、1280×800、1024×680 布局、状态、筛选、键盘和菜单回归 PASS |
| 最终缩略图改动 | `task-list-layout.browser.js` 4 组、`cover-copy-preview.browser.js` 13 组 PASS |
| 图片缓存 | `task-image-cache.browser.js`：切页复用、来源变化、进度不失效、并发读取合并、失败重试、移除释放 PASS |
| 菜单与动作 | `task-context-menu.browser.js` 5 组、`task-actions.browser.js` 12 组、`task-desktop-regression.browser.js` 3 组 PASS |
| 空状态 / Toast | `task-empty-feedback.browser.js` 三尺寸 PASS |
| Windows Tauri | 实际记录列表、新菜单、页面切换、铺满缩略图、上下箭头显示及点击滚动 PASS |

较早轮次的任务移除动画、历史清理、作者、转录弹窗、设置与关于页等回归见 [v3 验证](task-ui-v3-validation.md) 和 [v4 验证](task-ui-v4-validation.md)。本轮仅按受影响范围复核，没有将历史运行重新计为本轮测试。

本机运行日志和截图在被 Git 忽略的 `output/task-ui-v5/`、`output/task-ui-v6/`、`output/task-ui-v7/`，不将截图、真实任务记录或临时基线加入提交。最终原生截图为 `output/task-ui-v7/native-list.png`，菜单截图为 `output/task-ui-v6/native-menu.png`。

菜单首帧在隔离浏览器 31 条记录的三个尺寸中观测为 30/31/29ms；仅说明浏览器测试页面时序，不作为 Windows 全链路输入延迟或旧版性能对比。

## 验证边界

真实 Windows 验证只执行展示、页面切换、菜单打开关闭和滚动等无副作用操作；下载、删除、复制、转录等业务回归使用隔离 IPC fixtures。真实在线下载、真实文件删除、真实转录、macOS 和新安装包未在此次 UI 调整中执行。未修改 Rust/Python 下载引擎、SQLite 数据结构或受控业务 API。

按用户要求整理为本地提交，不推送。已有 `AGENTS.md` 改动和独立设计方案草稿保留在工作区，不纳入本次提交。
