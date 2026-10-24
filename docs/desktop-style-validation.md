# 浅色桌面下载器 UI 验证

2026-10-24，基于任务开始时已有的未提交视觉改造继续调整。本次没有提交或推送，没有修改 Rust/Python 下载、数据库或删除契约。

## 修改范围

- `src/main.tsx`：状态筛选置于顶栏，平台筛选置于统一列表容器顶部；与本地搜索取交集。点击行或聚焦行内操作显示浅蓝选中态。视频主操作显示“播放”，文案入口移至更多菜单。
- `src/style.css`：Radix Slate / Indigo 浅色外壳、较深的侧栏、白色列表与菜单、细分隔线；完成行 80px、辅助缩略图 56px，标题 14px / 600、元信息 12px。下载进度和错误仍可增加行高。
- `src/components/task-more-menu.tsx`、`src/components/video-transcript.tsx`：保留所有菜单操作及执行条件，关闭文案弹窗返回对应任务的更多按钮。
- `DESIGN.md`：同步本次颜色、材质、行高、筛选和菜单位置。
- 新增 `tests/desktop-task-filters.browser.js`；更新布局、操作与文案相关浏览器脚本的入口选择器和尺寸断言。原生文案脚本仅更新入口，本次没有执行。

下载中筛选包含已提交的解析任务；已取消记录仍在“全部”显示。作者搜索子视图不受任务状态筛选限制。历史清理继续采用原来的平台范围和确认流程，不随状态或关键词扩大/缩小删除语义。

## 验证结果

`npm run build`：PASS（TypeScript + Vite）。构建仍输出依赖 `use client` / sourcemap 提示和大 chunk 提示，无构建错误。

使用现有 Vite 服务和独立 Playwright Edge 会话，加载仓库隔离 IPC fixtures。以下命令均为 `playwright-cli -s=desktop-style --raw run-code --filename=tests/<脚本>`；没有读取真实 Cookie、调用真实播放器、下载平台作品或删除用户文件。

| 脚本 | 结果 | 覆盖 |
| --- | --- | --- |
| `desktop-task-filters.browser.js` | PASS，3 组 | 三尺寸状态/平台/关键词交集、实时事件、鼠标与键盘选中、不修改任务数据 |
| `task-list-layout.browser.js` | PASS，4 组 | 三尺寸连续行、80px 完成行、状态/操作居中、图集/缺失文件、菜单键盘、详情、滚动 |
| `task-actions.browser.js` | PASS，12 组 | 播放、复制、打开文件夹、两种删除语义、异常、忙碌、防重复、迟到事件、取消确认 |
| `task-platforms.browser.js` | PASS，8 组 | 原平台隔离、搜索、操作目标、导航保留、长列表滚动 |
| `visual-redesign.browser.js` | PASS，7 组 | 首页、任务、设置、关于、更新弹窗、媒体比例、减少动态效果、作者结果操作 |
| `clear-history.browser.js` | PASS，6 组 | 清理确认、空状态、异常、重试、忙碌及焦点 |
| `transcription.browser.js` | PASS，5 组 | 菜单入口、关闭焦点、三尺寸提取生命周期、复制/导出、旧事件、取消、文件缺失 |
| `transcription-model.browser.js` | PASS，5 组 | 历史模型标识、重新提取、关闭后重新打开 |
| `transcript-overflow.browser.js` | PASS，6 组 | 三尺寸长文案、连续英文、键盘滚动、固定操作区 |
| `nano-cuda.browser.js` | PASS，6 组 | 现有 CPU/CUDA 选择与保存结果展示的隔离 UI 回归 |
| `transcript-loading.browser.js` | PASS，6 组 | 三尺寸读取/识别阶段动画无溢出 |

`git diff --check`：PASS；新文件另行检查尾随空格、冲突标记和引用路径。

## 视觉结果

| 窗口 | 检查结果 | 截图 |
| --- | --- | --- |
| 1440 × 900 | PASS：展开侧栏、两层筛选、连续列表、选中态及菜单对齐 | [完成列表](../output/playwright/desktop-style/completed-1440.png)、[菜单](../output/playwright/desktop-style/menu-1440.png) |
| 1280 × 800 | PASS：列表列宽与文字层级稳定、无水平溢出 | [完成列表](../output/playwright/desktop-style/completed-1280.png)、[菜单](../output/playwright/desktop-style/menu-1280.png) |
| 1024 × 680 | PASS：图标侧栏、完整筛选与搜索、长标题省略、菜单未越界 | [完成列表](../output/playwright/desktop-style/completed-1024.png)、[菜单](../output/playwright/desktop-style/menu-1024.png) |

截图是隔离示例数据，保存在本机忽略目录 `output/`；不作为真实平台下载成功的证据。

## 验证边界

- 浏览器渲染与 IPC fixture 回归已验证；本次未启动 Tauri 原生窗口、未重新打包安装程序。
- Windows 原生 DPI 125% / 150%、macOS Intel / Apple Silicon 及真实下载/ASR：NOT_RUN。
- 原有工作区改动保留，审查范围按 `output/desktop-style-baseline/` 快照与 `output/desktop-style-manifest.json` 中的本次增量界定。
