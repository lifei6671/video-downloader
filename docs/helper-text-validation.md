# 说明文字层级统一（2026-10-22）

按用户要求，解释性、辅助性文字统一为 **12px / 400、`text-muted-foreground`**；正文仍为 14px。颜色复用 `DESIGN.md` 的 `on-surface-muted` / 现有主题 `#64748B`，不增加调色板、不通过透明度淡化。规则已写入根目录 `AGENTS.md` 的 UI 规范。

## 审查与修复

| 原有不一致 | 处理 |
| --- | --- |
| 字段帮助、卡片副标题与正文同为 14px | `FieldDescription`、`CardDescription` 默认改为 12px 常规字重、语义灰色。Cookie、代理、分块、UA、运行依赖及关于页共用此默认值。 |
| 普通说明提示、空状态、弹窗说明仍为 14px | `AlertDescription`、`EmptyDescription`、`DialogDescription`、`AlertDialogDescription`、`SheetDescription` 的默认说明样式统一；错误提示保留 destructive 等语义色。 |
| 页面/设置简介、运行依赖政策与缺失工具指引、Telegram 会话说明有独立 14px 段落 | 对应静态段落使用 `text-xs text-muted-foreground`。 |
| 已符合规范的辅助文字 | 关于页功能简介/项目链接描述/许可、工具路径、下载位置、技术提示等保持现有 12px 灰色。 |

审查覆盖全部生产 UI 及说明组件调用。主标题、字段标签、输入值、工具实际版本、任务计数/进度、作品正文/元数据和更新说明正文保持原有层级；Telegram 实际登录/保存状态段落保留原字号与角色。品牌主文案不作为表单辅助说明缩小。没有改动下载、代理凭据、缓存、队列、数据库或后端协议，也没有修改用户提供的设计资产。

独立审查发现，预览的队列位置也复用了 `CardDescription`，会被共享默认连带缩小。已为队列位置、作者、原声以及封面作品标题/图集作品概况设置明确的原 14px 例外，并增加实际 ready→queued 事件、作者/原声/封面标题的字号断言；这些是作品或状态数据，不能以说明文字处理。

## 实际验证

- `npm run build`：PASS，最终 TypeScript 检查与生产构建通过；保留已有构建告警。
- `npm run dev` + Playwright CLI / Edge：`tests/helper-text.browser.js` 使用新的隔离 `tests/fixtures/helper-text.html`，最终 `browser-helper-review-fix-final.log` 3 个视口组 PASS（1440×900、1280×800、1024×680）。实际 computed style 验证 12px / 400 / `rgb(100, 116, 139)`，覆盖下载/任务空态、三平台字段说明、分块、Telegram、运行依赖、关于页和两类弹窗。并验证输入值、标签、实际版本/任务计数、队列位置、作品作者/原声/封面标题和更新正文保持原字号，错误不被改为灰色。新增队列分支首轮因测试 fixture 的 StrictMode 旧订阅清理覆盖新订阅而超时；修正隔离订阅清理条件后通过，没有修改产品订阅或队列逻辑。
- 既有浏览器回归：代理回显/固定顶部 6 组、YouTube 分块 6 组、tdl 路径选择 11 组、任务平台 8 组，全部 PASS。收口任务计数字号后单独重跑任务平台 8 组通过。
- 检查三视口截图中的字号层级、换行、固定顶部和水平溢出；截图只有合成 Cookie、代理和工具路径，不使用用户截图中的真实凭据。
- 代码及样式之外没有行为变更，因此本轮不重复运行 Python、Rust 单元测试或 Clippy。Windows 预览构建与 sidecar 验证单独记录；浏览器结果不替代原生 GUI 人工验收或 macOS 验证。

日志、最终冻结源码清单及截图位于 `.local/helper-text-20261022/`。未自动提交、推送或发布，保留此前未提交改动与预览目录。

## Windows 预览

`APP_VERSION=1.7.1-preview.20261022.helper-text` 下执行 `npm run tauri -- build --features desktop --config D:/wx_lifeilin/github.com/lifei6671/video-downloader/.local/helper-text-20261022/tauri.preview.conf.json --no-bundle`：PASS，产物为 `output/Yingqu-1.7.1-preview.20261022-helper-text-win64/`。PE 确认为 x64 GUI、指定预览版本，生产资源包含新说明样式；`yingqu.exe` SHA-256 为 `002c427a2c8716468b07653e44feec6561264357bdc9057db87cfb18d00a12ae`。

Python 代码和依赖未变，复用上一预览包的完整 sidecar，逐文件哈希一致，未重新执行 PyInstaller。对新目录运行 `scripts/check_packaged_engine.py` 隔离 cwd/PATH 握手 PASS：引擎 0.7.5、yt-dlp 2026.08.19、JSONL v3。来源信息与全文件校验清单随包保存。未退出正在运行的旧预览；原生 GUI 字体体验由用户确认，macOS 未运行。

## 平台账号标题一致性补充（2026-10-22）

抖音、TikTok、YouTube 原本共用 `CardTitle` 的 500 字重，但中文回退字体与拉丁字体在该字重下的显示粗细不同。仅将平台账号卡片标题（含 Telegram）明确设为 `font-semibold` / 600，与设计规范的卡片标题字重一致；保留其他卡片、字段标签及 12px / 400 灰色辅助说明。

`npm run build` PASS；`tests/helper-text.browser.js` 增加当前活动 Tab 标题的实际字重断言，1440×900、1280×800、1024×680 三个视口组全部 PASS，覆盖四个平台切换及原有辅助说明检查。合成数据的抖音标题截图已核对。首轮新增测试因匹配了四个常驻 Tab 面板而触发严格模式错误，限定当前活动面板后通过；没有改动产品 Tab 行为。

本轮证据位于 `.local/platform-title-20261022/`。浏览器截图不替代原生 GUI 人工验收；未提交、推送或发布。
