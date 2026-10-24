# 下载任务行与右键菜单迭代

2026-10-24，用户在上一版基础上要求去除行间线、圆角悬停、纯图标播放，以及带显隐动画的右键菜单。

## 实现

- `src/style.css`：移除行间分隔线，行反馈背景为 8px 圆角；列表播放为 32px 无边框图标。任务菜单进入 150ms ease-out 淡入并轻移 3px，退出 120ms ease-out 淡出；关闭动画不拦截点击，减少动态效果时立即显隐。
- `src/components/task-more-menu.tsx`：复用同一组操作，任务列表通过 shadcn ContextMenu 整行右键打开；下载作品页继续使用原有 DropdownMenu。任务行支持 Shift+F10 和菜单键。封面的保存/复制菜单保持独立；菜单关闭、文案弹窗及移除任务的焦点返回分别处理。
- `src/main.tsx`：将任务行包入右键菜单，允许该区域的 contextmenu 事件；播放保留可访问名称和 Tooltip，去掉任务列表内的文字、边框和末尾更多图标。未改任务状态、API、Rust、Python、SQLite 或删除契约。
- `DESIGN.md`：同步为 2.3，更新任务行、入口、菜单动画和焦点规范。
- 相关现有浏览器测试改用右键入口，保留原有操作条件、业务回调及焦点断言；新增 `tests/task-context-menu.browser.js` 验证圆角、无分隔线、纯图标、动画、键盘、嵌套封面与无可用操作状态。

本轮基线及冻结文件清单：`output/task-ui-v4/baseline/`、`output/task-ui-v4/review-target.txt`。保留此前工作区修改，未提交或推送。

## 截图

| 尺寸 | 悬停 | 右键菜单 |
| --- | --- | --- |
| 1440×900 | [截图](../output/task-ui-v4/hover-1440.png) | [截图](../output/task-ui-v4/context-1440.png) |
| 1280×800 | [截图](../output/task-ui-v4/hover-1280.png) | [截图](../output/task-ui-v4/context-1280.png) |
| 1024×680 | [截图](../output/task-ui-v4/hover-1024.png) | [截图](../output/task-ui-v4/context-1024.png) |

以上运行真实 React 组件，使用隔离 IPC fixture。[真实 Windows Tauri 菜单截图](../output/task-ui-v4/native-context.png)读取现有任务；未操作真实下载、文案提取、移除或文件删除。macOS 与安装包本轮未验证。

## 检查结果

`npm run build`、`git diff --check`、设计 YAML/令牌引用及新增文件空白检查通过。构建仍有既有非阻塞 use-client/sourcemap 与大 chunk 提示，[日志](../output/task-ui-v4/build.log)。

隔离浏览器回归通过：

| 脚本 | 覆盖 |
| --- | --- |
| task-context-menu | 三尺寸视觉、进入/退出动画、键盘、焦点、封面嵌套、减少动态效果、无可用操作 |
| task-actions | 12 组播放/文件夹/复制/移除/删除、忙碌、失败、键盘与焦点 |
| task-list-layout | 三尺寸行高/列对齐、缩略图、详情、菜单、滚动 |
| task-removal-motion | 16 组移除生命周期、并发移除、失败、减少动态效果和焦点 |
| desktop-task-filters / task-desktop-regression / task-platforms | 状态、平台、搜索、事件、图集、取消重试、滚动 |
| clear-history | 原确认范围、错误、忙碌、焦点 |
| transcription / transcript-loading / transcript-overflow / transcription-model / nano-cuda | 入口、弹窗焦点、提取/取消、长文本、模型及 CUDA 状态 |
| cover-copy-preview | 封面复制/预览菜单保持独立，三尺寸及键盘 |
| visual-redesign | 其他页面与特殊动作布局 |

原生转录测试脚本只同步右键入口，本轮未执行真实转录。动画退场与任务移除曾导致焦点落到 body，已修复并通过相关回归。一次加载回归被开发热更新中断，已在代码冻结后重跑。
