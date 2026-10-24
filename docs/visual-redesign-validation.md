# 浅色轻玻璃视觉重构验证

日期：2026-10-24。对应 `DESIGN.md` 2.0，基线 HEAD 为 `20cd073b81983f578b2a132a62d1e2d8696ae05a`。前文保留首轮视觉重构的历史验证结果；后续任务页连续列表的当前尺寸和新增验证见末节。本记录不替代发布、全新安装或平台在线验收。

## 实现范围

- `src/style.css` 集中冻结 Radix light 色值、shadcn 语义变量、12/14/20px 字号、0.5px 边界、局部玻璃与控件状态；没有新增依赖。
- 首页使用自然比例大封面、同源弱模糊背景、图集浏览及规格选择。任务行最小 96px，下载中最小 128px，封面 72px；长内容和作者结果 ready 行允许增长。
- 高频动作直达，次要动作由 `TaskMoreMenu` 调用既有回调。移除记录保留文件，删除文件同步移除记录，均无二次确认；取消、重复下载、批量清理保留既有确认。
- 设置、关于、更新、视频文案和封面弹窗统一；多行项目链接与图集缩略图有独立尺寸，避免标准按钮规则覆盖。
- Tauri 配置使用 `theme: Light` 和珍珠白窗口背景，保留原生装饰与系统强调色。不引入透明窗口或新的 Mica/vibrancy 平台实现；轻玻璃通过内容层呈现并提供实色 fallback。

`src/api.ts` 的 SHA-256 仍为 `4642e65776217cde60dad7c9caf0fd2ed67fed3ce90a864087c5c54770fbd231`，与任务开始时一致。Rust/Python 实现、SQLite、协议、依赖清单和锁文件没有变更。上一阶段已有 `AGENTS.md` 修改及未跟踪参考方案均保留。

## 构建与静态检查

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| `npm run build` | PASS | TypeScript 检查与 Vite 生产构建完成 |
| `node --test tests/author-search.test.mjs tests/task-state.test.mjs tests/release-notes.test.mjs` | PASS | 8 项，包含作者身份、迟到 revision 和 Markdown 安全渲染 |
| YAML / Markdown / 令牌引用 | PASS | YAML 可解析、引用无缺失/循环，链接与组件尺寸核对，YAML palette/colors 与 CSS 原始变量和别名一致 |
| `git diff --check` | PASS | 包含本次新增文件的空白检查 |

构建仍输出既有的 `use client`/sourcemap 提示与大 bundle 提示，未作为本次视觉改动扩大构建或拆包范围。

## 浏览器交互回归

使用本机已安装的 Playwright CLI 与 Edge。测试 HTML 只在 `tests/fixtures/`，替换 IPC，不访问真实平台、凭据或用户媒体文件。生产入口不加载夹具。

已有 1420 服务及映取进程保持运行；本次另外启动 1421 服务。命令模式如下：

```powershell
npm run dev -- --port 1421
playwright-cli -s=yingqu-visual open http://127.0.0.1:1421/tests/fixtures/background-download.html?scenario=visual
playwright-cli -s=yingqu-visual run-code --filename tests/visual-redesign.browser.js --raw
```

| 脚本 | 结果与覆盖 |
| --- | --- |
| `visual-redesign.browser.js` | PASS：1440×900、1280×800、1024×680 四页、封面大图、自然比例、图集切换和选中标记、96/128px 行、真实 ETA、固定 Tabs、文件缺失、更新页脚、减少动效、作者 ready 行换行、项目支持链接不重叠 |
| `visual-scale.browser.js` | PASS：独立浏览器上下文模拟 125%/150% 像素比例；四页无横向溢出，文字只使用 12/14/20px。模拟不等于修改系统 DPI |
| `task-actions.browser.js` | PASS：12 组，菜单顺序/权限条件、键盘、原链接、记录/文件删除、错误保留、忙碌互斥、迟到事件及移除后焦点 |
| `background-download.browser.js` | PASS：15 组，后台交接、队列、下一次输入、重复下载确认与规格竞争 |
| `task-platforms.browser.js` | PASS：8 组，平台/搜索交集、队列位置、实时事件、长列表滚动、最后一行可达 |
| `clear-history.browser.js` | PASS：6 组，范围确认、记录保留/移除、失败、处理中与焦点 |
| `task-removal-motion.browser.js` | PASS：16 组，成功后退场、原位置收起、整批清理、失败不退场、减少动效、导航清理 |
| `platform-settings.browser.js` | PASS：10 组，平台草稿、手动/浏览器 Cookie、代理、读取失败的保留语义 |
| `ytdlp-variants.browser.js` | PASS：规格切换、同规格/历史未知规格/文件缺失、完成竞争、ETA/估算/合并及三个视口 |
| `transcription.browser.js` | PASS：5 组，单视频提取、取消、旧结果、复制/导出、依赖下载后显式开始 |
| `transcript-overflow.browser.js` | PASS：6 组，三视口长正文/连续字符、滚动、固定操作区和完整导出 |
| `cover-preview-fit.browser.js` | PASS：10 组，横竖大图、长标题、无多余滚动、原图复制、另存为、失败、键盘和焦点 |
| `cover-copy-preview.browser.js` | PASS：13 组，六类封面原尺寸 RGBA、失败/忙碌、缺图、三视口及关闭/遮罩 |
| `runtime-dependencies.browser.js` | PASS：7 组，固定工具栏、各依赖独立下载/取消、CPU/CUDA 展示、模型大小与未知状态 |
| `tdl-picker.browser.js` | PASS：11 组，选择/保存/检测、同路径重检、失败保留、忙碌保护、手动 Enter 和跨页保留 |

旧测试中“行尾所有按钮平铺”和中点分隔文案按本版规范更新；保留文件副作用、确认、失败、事件及身份断言。焦点断言等待 Radix 的真实异步恢复，不强制设置焦点冒充通过。

## Windows 原生检查

- PASS：系统版本 `Microsoft Windows 10.0.26200`，WebView2 `Edg/150.0.4078.65`，实际 `devicePixelRatio=1.5`，内容视口 1440×900。
- PASS：独立编译并运行 Tauri debug 实例，标识为 `me.disign.yingqu.visual-validation-20261024`。先创建独立空数据目录，避免触发旧目录迁移；WebView 数据也隔离，原有用户实例未退出、覆盖或改动。
- PASS：`native-visual.browser.js` 通过真实 IPC 检查空任务数据库与未配置凭据的设置，完成四页导航、运行依赖与关于页展示，没有发起平台下载或登录。
- PASS：原生最小化/最大化/关闭控件保持；实测关闭本次窗口后从可见窗口列表消失，再次启动同一独立实例恢复同一窗口 ID，验证关闭隐藏及单实例恢复。

默认 debug exe 被既有进程占用，未强行结束。实际构建使用独立目录：

```powershell
# 编译时用 TAURI_CONFIG 指定上述独立标识和 1421 devUrl。
cargo build --manifest-path src-tauri/Cargo.toml --target-dir .local/visual-redesign/native-target --features desktop --bin yingqu
```

原生测试覆盖当前 150% 系统缩放。125% 仅进行了浏览器模拟，没有修改用户的系统缩放设置。原生箭头按钮的实际点击滚动未单独验收。

## 截图与未执行项目

截图位于 `output/playwright/visual-redesign/`：`home-*`、`tasks-*`、`settings-*`、`about-*`、`update-*` 覆盖三个视口，`scale-*` 是像素比例模拟，`native-home.png` / `native-about.png` 来自真实 Tauri WebView。

- NOT_RUN：macOS Intel、Apple Silicon 实机视觉/窗口测试。配置仍声明最低 macOS 13.3，未用 Windows 成功代替 macOS 结果。
- NOT_RUN：全新安装、发行安装包、真实平台在线下载、真实用户文件删除、依赖实际安装及 Telegram 扫码/2FA。本次相应回调与条件由隔离交互回归覆盖，未将 mock 当作在线或安装验收。
- 未新增原生系统玻璃 API；保留现有原生窗口行为，材质不足时回退实色。

## 下载任务页连续列表调整（2026-10-24）

本次在已存在的未提交视觉重构上继续调整任务列表，不重置工作区。任务行改为统一容器内的连续排列，常规行 88px、封面 64px、状态列 104px、操作列 144px；下载中最小 120px，等待和错误反馈按内容增长。作者头像 20px，平台与作者同行，标题单行省略且可通过键盘打开任务详情，完整标题、作者/作品/任务 ID、路径及时间仍可访问。首页作者资料默认行为保持。`AGENTS.md` 只同步原有 96px 条款，避免与现行 `DESIGN.md` 冲突。

`TaskMoreMenu` 的回调和执行条件未修改；任务接口、状态归并、Rust/Python、数据库和窗口配置未改动。本次 `src/api.ts` 哈希仍与前文一致。

| 检查 | 结果 |
| --- | --- |
| `npm run build` | PASS，TypeScript 与 Vite 构建完成，既有 bundle 提示仍在 |
| YAML / Markdown / CSS 令牌 | PASS，YAML 无重复键，101 次令牌引用检查无缺失/循环，6 个任务尺寸映射一致，13 个表格结构及本地链接有效 |
| `git diff --check` / 新文件尾随空白 | PASS |
| `task-list-layout.browser.js` | PASS，1440×900、1280×800、1024×680：连续排列、88px 常规行、状态/操作居中、真实进度/ETA、长/缺失标题、详情全文/字段/焦点、菜单右对齐/键盘、多条已完成、文件缺失、图集、无封面、自然比例与列表滚动；另验收 1024 下超长详情正文纯键盘滚动与关于页复选清单样式隔离 |
| `task-actions.browser.js` | PASS，12 组，播放/复制/文件夹、动作条件、移除/删除、忙碌/失败/迟到事件、焦点及取消确认 |
| `task-platforms.browser.js` | PASS，8 组，筛选、搜索、队列顺序、事件与固定 Tabs；无标题期望更新为真实来源/任务标识 |
| `task-removal-motion.browser.js` | PASS，16 组，连续列表中的成功退场、后续行位移、批量清理与减少动态效果 |
| `clear-history.browser.js` | PASS，6 组，确认范围、失败/处理中和最终焦点 |
| `author-profile.browser.js` | PASS，6 类作者资料与三个视口，头像回退和搜索保持；测试地址改为当前会话 origin，取消固定端口依赖 |
| `visual-redesign.browser.js` | PASS，三个视口四页与弹窗回归、自然比例/图集、作者 ready 行操作换行；任务行断言更新为 88/120px |
| `cover-copy-preview.browser.js` | PASS，13 组，封面原尺寸复制、缺失/失败、预览、键盘与焦点 |

截图位于 `output/playwright/task-list-layout/`，`mixed-*`、`completed-*`、`menu-*`、`details-*` 分别记录混合状态、多条完成记录、展开菜单和完整详情，均覆盖三个视口。已检查截图实际构图和对齐。以上浏览器测试使用真实 React 界面和隔离 IPC 样例，不操作真实下载、凭据或用户文件。

本轮 NOT_RUN：Tauri 原生窗口和 macOS 实机复测、真实在线下载与真实文件删除；不能将前文首轮原生测试视为本轮新的运行证据。未提交或推送。
