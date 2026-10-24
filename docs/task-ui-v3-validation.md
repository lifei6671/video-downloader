# 下载任务页高保真重构验证

2026-10-24，本地工作区，未提交、推送或发布。视觉依据为用户提供的迅雷桌面截图及本轮任务规格。

## 改动范围

| 文件 | 本轮修改 |
| --- | --- |
| `src/main.tsx` | 保留任务订阅、修订归并、操作调度和弹窗；恢复可见页标题，派生筛选计数，组合列表展示组件 |
| `src/components/task-list.tsx` | 新增连续列表、平台筛选、空状态与滚动结构，继续使用原 TaskRemovalRow 生命周期 |
| `src/components/task-row.tsx` | 新增四列任务行，状态与操作居中，队列位置、错误和封面反馈 |
| `src/components/task-filters.tsx` | 新增受控状态导航及平台展示，计数来自父组件实际任务集合 |
| `src/components/task-more-menu.tsx` | 复用已有菜单和回调，添加任务菜单样式作用域；未为改名另建重复组件 |
| `src/components/author-info.tsx` | 新增默认开启的 showAvatar，任务行只显示作者文字，其他页面保持头像和 ID 行为 |
| `src/style.css` | 连续白色工作区、双层筛选、两级任务信息、固定列宽、选中态、140ms 背景过渡、224px 菜单 |
| `DESIGN.md` | 同步到 2.2，更新对应布局、尺寸、信息层级、状态计数、菜单规格及交互；保留其他页面业务规则 |
| `tests/desktop-task-filters.browser.js` | 增加真实派生计数断言，适配计数后的可访问名称与背景过渡 |
| `tests/background-download.browser.js` | 队列位置断言改为新的状态列，保留队列行为检查 |
| `tests/task-desktop-regression.browser.js` | 新增二层信息、菜单密度、进度/完成事件、页面切换、图集、取消、重试回归 |
| `tests/fixtures/background-download.html` | 在隔离 fixture 中补充 retry 模拟，未触及生产 API |
| 本验证文档 | 记录真实桌面与隔离浏览器证据及边界 |

本轮开始前的相关文件保存在 `output/task-ui-v3/baseline/`。工作区原有 AGENTS、Tauri 配置、其他组件和测试修改均保留；本轮未修改 Rust、Python、SQLite、`src/api.ts`、依赖或 TaskRemovalRow 实现。

## 视觉结果

- 192px 浅灰蓝侧栏与白色工作区分层；原四个导航、应用品牌、本地运行及窄窗折叠保留。
- 顶部可见 20px 标题、搜索与清理入口；下方为 48px 状态导航和 52px 紧凑平台导航。状态数量匹配当前平台和搜索条件，不受所选状态限制，退出动画快照不计入数量。
- 任务行四列为 56px / 弹性信息 / 88px / 144px，间距 12px。已完成 80px，等待 84px，下载中至少 120px；失败和异常允许扩高。
- 普通任务只有标题与元信息两层。长标题省略，完整信息由原详情入口查看；作者、来源、规格和大小同一行，文件缺失仍显示红色提示。
- 保持原比例缩略图、横版/竖版/图集与不可用占位。选中和菜单打开使用整行 Radix Indigo 3 浅蓝背景；沿用原 Radix Slate/Indigo/Jade/Amber/Red 语义色，无新增调色板。
- 更多菜单宽 224px、条目最小高 36px、圆角 12px；危险动作红字和红图标，统一浅蓝高亮。没有添加未实现快捷键或迅雷专属功能。

### Windows Tauri 真实截图

启动方式：`npm run desktop -- --config D:/wx_lifeilin/github.com/lifei6671/video-downloader/output/desktop-style-launch.json`，临时配置仅跳过已运行的 Vite 启动命令。实际窗口为本地 debug `yingqu.exe`，真实读取现有 68 条已完成记录，含文件缺失和横竖封面。

通过系统窗口菜单调整大小，保留原生边框。以下尺寸指内容区；工具捕获的含原生边框逻辑尺寸分别为 1443×931、1283×831、1027×711，与原始 Tauri 1440×900 配置及原生边框增量一致。未使用静态原型或浏览器截图冒充桌面窗口。

| 内容尺寸 | 列表 | 更多菜单 | 结果 |
| --- | --- | --- | --- |
| 1440×900 | [真实列表](../output/task-ui-v3/native-1440.png) | [真实菜单](../output/task-ui-v3/native-menu-1440.png) | PASS，展开侧栏、连续任务、对齐与选中状态 |
| 1280×800 | [真实列表](../output/task-ui-v3/native-1280.png) | [真实菜单](../output/task-ui-v3/native-menu-1280.png) | PASS，信息与操作未重叠，浮层位于窗口内 |
| 1024×680 | [真实列表](../output/task-ui-v3/native-1024.png) | [真实菜单](../output/task-ui-v3/native-menu-1024.png) | PASS，侧栏收起，无可见整体水平溢出，菜单及焦点恢复正常 |

桌面只执行页面查看、选择、任务详情、菜单打开/Escape 和窗口调整，没有播放、下载、提取、清理或删除真实文件。原生系统播放、真实在线下载/排队/取消/重试、真实图集和文案提取本轮 NOT_RUN；其前端入口与状态回归使用下述隔离 IPC。macOS 和安装包本轮 NOT_RUN。

### 浏览器实际组件渲染

运行真实 React 组件，IPC 为明确隔离的 fixture，覆盖下载中、等待、失败、图集、缺失文件、封面缺失、长标题等状态；不是静态效果图。

| 视口 | 完成列表与选中态 | 菜单展开 | 几何检查 |
| --- | --- | --- | --- |
| 1440×900 | [截图](../output/task-ui-v3/completed-1440.png) | [截图](../output/task-ui-v3/menu-1440.png) | PASS |
| 1280×800 | [截图](../output/task-ui-v3/completed-1280.png) | [截图](../output/task-ui-v3/menu-1280.png) | PASS |
| 1024×680 | [截图](../output/task-ui-v3/completed-1024.png) | [截图](../output/task-ui-v3/menu-1024.png) | PASS |

[混合状态](../output/playwright/task-list-layout/mixed-1024.png)、[任务详情](../output/playwright/task-list-layout/details-1024.png)。几何检查包括连续行无间隙、固定列对齐、80/84px 行高、56px 封面、单行元信息、滚动中标题/筛选固定、菜单边界与键盘焦点。

## 工程与交互回归

`npm run build`：PASS，TypeScript 与 Vite 构建完成。保留已有 use-client/sourcemap 和 chunk 超过 500kB 的非阻塞提示；[构建日志](../output/task-ui-v3/build.log)。`git diff --check`：PASS；新文件另外检查行尾空格及文档引用。

执行方式：已有 Vite 开发服务，`playwright-cli -s=task-ui-v3 --raw run-code --filename=tests/<名称>.browser.js`。`background-download` 脚本从其对应 fixture 页面运行。

| 测试脚本 | 结果 | 重点覆盖 |
| --- | --- | --- |
| task-list-layout | PASS，4 组 | 三尺寸行列、媒体比例、缺失/图集/长标题、详情、菜单及滚动 |
| desktop-task-filters | PASS，3 组 | 状态/平台/搜索交集、计数、事件更新、选中、键盘、不修改任务 |
| task-desktop-regression | PASS，3 组 | 二层信息、84px 队列、菜单密度、75% 进度事件、完成跨页面同步、图集、取消、重试 |
| task-actions | PASS，12 组 | 播放/文件夹/链接、菜单条件、删除/移除区分、失败与忙碌、焦点 |
| task-removal-motion | PASS，16 组 | 保留 data-task-row/data-removing/inert/onExited、同时移除、失败不移除、迟到事件、减少动态效果 |
| background-download | PASS，15 组 | 排队、后台提交、确认、错误、忙碌、迟到事件及页面输入状态 |
| author-profile | PASS，6 场景及三尺寸 | 作者/频道信息、缺失与加载失败 |
| cover-copy-preview | PASS，13 组 | 封面预览与复制、失败、忙碌、键盘和三尺寸 |
| cover-preview-fit | PASS，10 组 | 横竖封面、保存/另存为、取消、图片菜单与焦点 |
| task-platforms | PASS，8 组 | 平台隔离、FIFO、搜索、事件、长列表滚动与页面切换 |
| clear-history | PASS，6 组 | 原确认范围、失败保留、忙碌和焦点 |
| transcription | PASS，5 组 | 文案入口、结果、复制/导出、取消、迟到事件与文件缺失 |
| visual-redesign | PASS，7 组 | 下载/任务/设置/关于、更新弹窗、媒体比例、特殊长动作换行 |

验证中两处测试同步：旧队列完整句改为状态列队列位置；状态键盘断言改用稳定可访问名称而非包含计数的 textContent。均保留行为断言，没有通过删除有效断言绕过失败。

## 审查与范围

本轮独立子 Agent 只读审查了冻结的组件、样式、主组件组合、测试和设计规范，并查看三尺寸截图。未发现 P0/P1 或操作契约回归；菜单危险项聚焦底色的一处文档不一致已同步。冻结文件校验记录位于 `output/task-ui-v3/review-target.txt`。未自动提交或推送。
