---
version: "2.3"
name: "映取"
description: "Windows / macOS 精致型影音下载器，浅色桌面下载器设计规范。"
theme: light
palette:
  white: "#ffffff"
  slate-1: "#fcfcfd"
  slate-2: "#f9f9fb"
  slate-3: "#f0f0f3"
  slate-4: "#e8e8ec"
  slate-5: "#e0e1e6"
  slate-6: "#d9d9e0"
  slate-7: "#cdced6"
  slate-8: "#b9bbc6"
  slate-11: "#60646c"
  slate-12: "#1c2024"
  slate-a3: "#0000330f"
  slate-a4: "#00002d17"
  slate-a9: "#00051d74"
  indigo-3: "#edf2fe"
  indigo-4: "#e1e9ff"
  indigo-5: "#d2deff"
  indigo-8: "#8da4ef"
  indigo-9: "#3e63dd"
  indigo-10: "#3358d4"
  indigo-11: "#3a5bc7"
  indigo-12: "#1f2d5c"
  cyan-3: "#def7f9"
  cyan-12: "#0d3c48"
  violet-3: "#f4f0fe"
  violet-12: "#2f265f"
  orange-3: "#ffefd6"
  orange-12: "#582d1d"
  jade-3: "#e6f7ed"
  jade-11: "#208368"
  jade-12: "#1d3b31"
  amber-3: "#fff7c2"
  amber-11: "#ab6400"
  amber-12: "#4f3422"
  red-3: "#feebec"
  red-4: "#ffdbdc"
  red-11: "#ce2c31"
  red-12: "#641723"
colors:
  background: "{palette.slate-2}"
  sidebar: "color-mix(in srgb, {palette.slate-2} 60%, {palette.slate-3})"
  surface: "{palette.white}"
  surface-raised: "{palette.white}"
  surface-input: "{palette.slate-1}"
  surface-subtle: "{palette.slate-2}"
  surface-control: "{palette.slate-3}"
  surface-hover: "{palette.slate-4}"
  surface-pressed: "{palette.slate-5}"
  surface-selected: "{palette.indigo-3}"
  primary: "{palette.indigo-11}"
  primary-text: "{palette.indigo-12}"
  on-primary: "{palette.slate-1}"
  primary-hover: "color-mix(in srgb, {palette.indigo-11} 88%, {palette.indigo-12})"
  primary-pressed: "color-mix(in srgb, {palette.indigo-11} 76%, {palette.indigo-12})"
  primary-tint: "{palette.indigo-3}"
  primary-tint-hover: "{palette.indigo-4}"
  primary-tint-pressed: "{palette.indigo-5}"
  on-surface: "{palette.slate-12}"
  on-surface-secondary: "{palette.slate-11}"
  on-surface-muted: "{palette.slate-11}"
  border: "{palette.slate-4}"
  border-input: "{palette.slate-7}"
  border-strong: "{palette.slate-8}"
  focus-ring: "{palette.indigo-11}"
  success: "{palette.jade-11}"
  success-bg: "{palette.jade-3}"
  success-text: "{palette.jade-12}"
  warning: "{palette.amber-12}"
  warning-bg: "{palette.amber-3}"
  warning-text: "{palette.amber-12}"
  error: "{palette.red-11}"
  error-bg: "{palette.red-3}"
  error-hover: "{palette.red-4}"
  error-text: "{palette.red-12}"
  info-bg: "{palette.indigo-3}"
  info-text: "{palette.indigo-12}"
  progress-track: "{palette.slate-4}"
  progress-value: "{palette.indigo-11}"
  progress-glass-background: "color-mix(in srgb, {colors.surface-raised} 78%, transparent)"
  media-placeholder: "{palette.slate-3}"
  scrollbar-thumb: "{palette.slate-7}"
  scrollbar-thumb-hover: "{palette.slate-12}"
  backdrop: "{palette.slate-a9}"
typography:
  font-family: '"Alibaba PuHuiTi 3.0", system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif'
  font-mono: 'ui-monospace, "SFMono-Regular", Consolas, monospace'
  title-page: { fontSize: 20px, fontWeight: 600, lineHeight: 28px }
  title-section: { fontSize: 14px, fontWeight: 600, lineHeight: 20px }
  title-card: { fontSize: 14px, fontWeight: 600, lineHeight: 20px }
  body: { fontSize: 14px, fontWeight: 400, lineHeight: 22px }
  body-emphasis: { fontSize: 14px, fontWeight: 600, lineHeight: 22px }
  label: { fontSize: 14px, fontWeight: 500, lineHeight: 20px }
  button: { fontSize: 12px, fontWeight: 500, lineHeight: 18px }
  caption: { fontSize: 12px, fontWeight: 400, lineHeight: 18px }
  telemetry: { fontSize: 12px, fontWeight: 500, lineHeight: 18px }
spacing:
  xxs: 4px
  xs: 8px
  sm: 12px
  md: 16px
  lg: 20px
  xl: 24px
  xxl: 32px
  section: 32px
  sidebar-width: 192px
  sidebar-compact-width: 56px
  content-padding: 24px
  content-padding-compact: 16px
  content-max: none
rounded:
  xs: 4px
  sm: 6px
  md: 8px
  lg: 12px
  xl: 16px
  full: 9999px
borders:
  width: 0.5px
  focus-width: 2px
  focus-offset: 2px
materials:
  media-atmosphere:
    opacity: 0.16
    blur: 20px
    background: "{colors.media-placeholder}"
  shadow-menu: "0 8px 24px {palette.slate-a3}, 0 2px 6px {palette.slate-a4}"
  shadow-task-menu: "0 12px 32px {palette.slate-a3}"
  shadow-dialog: "0 16px 48px {palette.slate-a4}"
motion:
  duration: 250ms
  easing: ease-out
  removal-duration: 220ms
components:
  app-shell: { brandHeight: 64px, brandIconSize: 32px, headerMinHeight: 56px }
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    typography: "{typography.button}"
    rounded: "{rounded.md}"
    height: 32px
    paddingInline: "{spacing.sm}"
  button-secondary:
    backgroundColor: "{colors.surface-control}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.md}"
    height: 32px
  button-ghost:
    backgroundColor: transparent
    textColor: "{colors.on-surface-secondary}"
    rounded: "{rounded.md}"
    height: 32px
  button-small: { height: 28px, rounded: "{rounded.sm}" }
  button-icon: { width: 32px, height: 32px, iconSize: 16px }
  button-multiline: { minHeight: 56px, padding: "{spacing.xs}" }
  input-default:
    height: 32px
    backgroundColor: "{colors.surface-input}"
    textColor: "{colors.on-surface}"
    borderColor: "{colors.border-input}"
    rounded: "{rounded.md}"
  input-compact: { height: 28px }
  share-input: { rows: 3, height: 80px, lineHeight: 20px, padding: "{spacing.xs}" }
  card: { rounded: "{rounded.lg}", padding: "{spacing.md}" }
  task-list: { rounded: 0px, rowGap: 4px, backgroundColor: "{colors.surface}", hoverBackgroundColor: "{colors.surface-subtle}" }
  task-feed: { minColumnWidth: 240px, columnGap: 16px, rowGap: 20px, coverRatio: "3 / 4", titleLines: 2 }
  video-player: { maxWidth: 960px, stageHeight: "min(56dvh, 480px)" }
  navigation: { height: 40px, rounded: "{rounded.md}", iconSize: 16px }
  chip-neutral: { height: 24px, rounded: "{rounded.full}" }
  preview:
    mediaFraction: 0.4
    infoFraction: 0.6
    stageHeight: 360px
    stageHeight-medium: 320px
    stageHeight-compact: 300px
    rounded: "{rounded.xl}"
    gap: "{spacing.xl}"
    formatsMaxHeight: 196px
    formatMinHeight: 56px
  task-toolbar: { minHeight: 64px, statusHeight: 48px, platformHeight: 52px, searchWidth: 240px }
  task-row:
    minHeight: 80px
    activeMinHeight: 120px
    coverSize: 56px
    queuedMinHeight: 84px
    statusWidth: 88px
    actionWidth: 144px
    gap: "{spacing.sm}"
    paddingBlock: "{spacing.xs}"
    paddingInline: "{spacing.md}"
    rounded: "{rounded.md}"
  progress: { height: 4px, rounded: "{rounded.full}", circularSize: 64px, circularTrackWidth: 4px }
  task-menu: { width: 236px, itemHeight: 36px, padding: 8px, rounded: 10px }
  menu: { minWidth: 192px, itemHeight: 32px, padding: "{spacing.xxs}", rounded: "{rounded.lg}" }
  dialog: { width: 480px, maxHeight: 85dvh, viewportGap: 16px, padding: "{spacing.lg}", rounded: "{rounded.xl}" }
  transcript-dialog: { width: 720px, maxHeight: 85dvh }
  update-dialog: { width: 640px, maxHeight: 85dvh }
  album-dialog: { width: 680px, maxHeight: 90dvh }
  cover-dialog: { maxWidth: 90vw, maxHeight: 90dvh }
  avatar: { size: 32px, compactSize: 24px }
  album-thumbnail: { width: 40px, height: 48px }
  settings: { navigationWidth: 184px, navigationWidth-compact: 160px, gap: "{spacing.xl}" }
  about: { maxWidth: 1040px }
  scrollbar: { size: 6px, contentGap: 12px, aboutOuterGap: 4px }
---

# 映取桌面客户端设计规范

> 版本 2.3，2026-10-24。适用于 Windows / macOS，界面语言为简体中文，基准视口为 1440 × 900。
>
> 本文件是前端实现与后续维护的唯一视觉规范，包含应用页面的布局、组件规格、媒体交互和有效业务规则。顶部 YAML 定义唯一视觉令牌，正文解释用途；数值如有冲突，先修正文档再实施。本版已应用到 React/shadcn 界面，下载器调整见 [验证记录](docs/task-ui-v4-validation.md)，知识库进度见第 12 节，上一版记录见 [视觉重构验证记录](docs/visual-redesign-validation.md)。

## 1. 产品定位与规范边界

### 1.1 精致型影音下载器

映取帮助普通用户、内容整理者和创作者将喜欢的作品保存到本地。核心路径是：粘贴单条分享链接 → 解析作品 → 识别封面与作者 → 选择真实可用规格 → 下载 → 管理本地作品与记录。

视觉由珍珠白内容表面、雾蓝灰结构、清透蓝交互强调和真实媒体封面组成。作品识别优先于技术参数，下载操作优先于装饰，过程反馈优先于空泛说明。分享链接、封面、图集原图、等待队列、本地文件夹构成产品的主要视觉语境。

- 首页以大封面舞台识别作品，任务页使用统一容器内的连续媒体列表，常规行高 80px；标题、作者、来源与文件信息形成稳定的阅读顺序。
- 固定导航和顶栏采用浅灰实色，列表与表单保持实色；通过色阶、留白和 0.5px 边界区分结构。
- 行尾保留高频动作，次要操作通过整行右键菜单访问，避免一排无层级的操作图标。
- 全局只有一种蓝色交互体系；抖音、TikTok、YouTube、Telegram 使用中性来源标签，不切换应用主色。
- 应用名称保持“映取”。不用营销横幅、数据看板、大插画、彩色左边线或装饰性眉题制造层次。

### 1.2 已有能力与实施边界

正式支持抖音、TikTok、YouTube 单作品；图集沿用已支持的平台和引擎能力。Telegram 原始视频是默认关闭的 tdl HTTP 实验模式，不能作为无条件可用的第四个正式平台宣传。

前端继续复用 Tauri 2、React、TypeScript、Tailwind CSS 4、shadcn/ui radix-nova、Radix 与 Lucide。所有基础控件复用 `src/components/ui/`，页面负责布局和业务组合。设计中的组件名称表示展示职责，不要求逐个新建文件。

Rust 仍是任务、队列、文件提交与持久化的唯一状态权威，`src/api.ts` 是前端调用边界。重构保持既有事件订阅、`revision` 合并、移除屏蔽、配置快照、取消清理与恢复语义。不改 Rust、Python、SQLite、协议或下载逻辑来配合外观。

不新增平台内容搜索、作者批量采集、播放列表、直播、转码、暂停、断点续传、账号注册、付费套餐、云同步、开放 API。MCP 仅提供本机已收录素材访问，权限见素材库及 MCP 专项契约。作者昵称进入的是本地记录搜索；视频优先在应用内播放，不支持时调用系统默认播放器。

### 1.3 权威关系与历史整理

当前用户指令决定任务范围；本文件决定视觉和下述界面行为，技术方案与专项契约说明底层实现。功能冲突须记录并核对，不能借重构改变契约。`docs/ui/` 的旧稿只帮助辨认页面状态和内容，旧颜色、字号、密度及控件位置不约束新界面。

旧版按 M2 任务卡追加的要求已合并到对应章节，不再保留另一套阶段性视觉规则：

| 旧记录 | 本版有效规则 |
| --- | --- |
| M2 单任务、并发、历史管理 | 保留队列、重试、取消和文件操作，见第 5、6 节；不再把已实现操作当作未来按钮 |
| 紧凑密度、固定 9:16 小封面 | 改为本版大媒体舞台与 80px 连续任务行，所有源图保持自然比例 |
| 行尾全部平铺、取消更多菜单 | 由本次要求覆盖为高频直达和整行右键菜单，执行条件与副作用不变 |
| ToggleGroup 并发选择 | 当前使用 Select，默认 2、可设 1–3；不恢复旧控件要求 |
| 双平台文案、独立已完成导航 | 导航按第 3.1 节；已完成仍在下载任务内，平台范围见 1.2 |
| 本地封面与视频同目录保存 | 新封面只存现有 SQLite 缓存；显式保存封面仍可用，见第 7 节 |
| 历史二次确认删除 | 单条移除与本地文件删除均直接执行；取消、重复下载及批量清理保留各自确认 |
| 黑色滚动条悬停、旧阴影与多档字号 | 统一使用本版 Radix、材质与三档字号令牌 |

## 2. 设计令牌与样式映射

### 2.1 Radix 浅色色阶

YAML `palette` 记录本版使用的 Radix Colors light sRGB 色值，`colors` 为语义别名。来源为 [Radix 浅色源码](https://github.com/radix-ui/colors/blob/main/src/light.ts)，用法依据 [色阶用途](https://www.radix-ui.com/colors/docs/palette-composition/understanding-the-scale)。不从封面自动提取应用强调色，不在页面加入另一套近似灰蓝或状态色。

| 用途 | 语义令牌 | Radix 来源与使用方式 |
| --- | --- | --- |
| 窗口内容底色、侧栏基底 | `background`、`sidebar` | slate-2；侧栏混入 40% slate-3，略深于主背景 |
| 内容、输入、浮层 | `surface`、`surface-input`、`surface-raised` | white，列表与浮层为白色；输入使用 slate-1 |
| 次表面 | `surface-subtle` | slate-2，与内容轻微区分 |
| 控件默认、悬停、按下 | `surface-control`、`surface-hover`、`surface-pressed` | slate-3 / 4 / 5 |
| 导航选中、弱强调及其交互 | `surface-selected`、`primary-tint` 系列 | indigo-3 / 4 / 5，文字使用 `primary-text` 的 indigo-12 |
| 普通分隔、输入、悬停边界 | `border`、`border-input`、`border-strong` | slate-4 / 7 / 8 |
| 标题正文、次级及辅助文字 | `on-surface`、`on-surface-secondary`、`on-surface-muted` | slate-12 / 11 / 11，以字号和字重区分次级层次 |
| 主按钮、链接、进度、焦点 | `primary`、`progress-value`、`focus-ring` | indigo-11；主按钮文字使用 slate-1 |
| 已完成 | `success-bg` + `success-text` | jade-3 + jade-12 |
| 等待、取消中、实验风险 | `warning-bg` + `warning-text` | amber-3 + amber-12 |
| 失败、危险操作 | `error-bg` + `error-text` | red-3 + red-12；独立错误文字用 `error` |
| 解析中、下载中、已解析 | `info-bg` + `info-text` | indigo-3 + indigo-12 |
| 遮罩、阴影 | `backdrop`、`materials.shadow-*` | slate alpha 色阶 |

普通文本及按钮文字在实际背景上至少达到 WCAG AA 4.5:1。indigo-9 / 10 保留为原始色阶，不直接用来承载本项目 12px 浅色按钮文字；主按钮使用 indigo-11，hover / pressed 按 YAML 混入 indigo-12 加深，不通过降低整个按钮透明度变浅。导航与筛选使用 indigo-11 文字配 indigo-3 背景，各类状态徽标使用对应第 12 阶文字。焦点除颜色外还具有独立轮廓。

媒体像素和应用图标原始素材不受界面调色板重着色；Telegram 二维码保持足够扫描对比。除这些内容外，所有界面色、渐隐、阴影及 SVG 滚动箭头均来自令牌。状态色只表达真实状态。

### 2.2 字体、间距和形状

| 层级 | YAML 角色 | 规格与用途 |
| --- | --- | --- |
| 页面级 | `title-page` | 20px / 600 / 28px，主页面标题和关于页品牌名称 |
| 正文级 | `body`、`label`、`title-section`、`title-card` | 14px，正文 400、标签 500、标题 600；行高按 YAML |
| 辅助级 | `caption`、`telemetry`、`button` | 12px，说明 400、数值与按钮 500，行高 18px |

全局只有 12 / 14 / 20px 三档字号；弹窗标题、更新说明、徽标、格式信息、作者 ID 不增加第四档，也不使用 11px。分组与任务标题使用 14px / 600。全局优先使用随应用打包的阿里巴巴普惠体 3.0，按 400 / 500 / 600 分别加载 Regular / Medium / SemiBold，缺字回退本机系统字体；不加载远程字体，不修改或裁减原始字库，Bold 暂不打包。数值使用 `tabular-nums`，技术标识需要时才用 `font-mono`。

辅助说明统一为 12px 常规字重和 `text-muted-foreground`，优先复用 `FieldDescription`、`CardDescription`。作品正文、标题、字段标签、用户输入、更新说明正文和实际进度数值不属于辅助说明。错误、警告、成功保留语义色。界面不使用中点分隔文案，多项信息通过间距、换行或完整短句组织。

间距使用 4 / 8 / 12 / 16 / 20 / 24 / 32px。正文常规内边距 24px，窄窗 16px；常规分组内边距 16px，表单字段间距 12px。避免为每个字段另套卡片。

圆角按功能使用：小控件 6px、按钮和输入 8px、分组与菜单 12px，任务列表不使用外层圆角、大舞台和弹窗 16px；任务行悬停、选中背景为 8px 圆角。4px 仅用于紧邻的小型内部元素，胶囊只用于状态和进度。嵌套时内圆角约为外圆角减去间距并限定为非负值，不给每层相同的大圆角。

所有结构边框、分隔线、菜单分隔和表格线为 0.5px。2px 焦点轮廓属于可访问反馈，不是结构边框；Lucide 图标保留统一的 1.75–2px 线宽。避免再叠 shadcn 默认 1px ring 和自定义边框形成双线。

### 2.3 材质、阴影与动效

| 区域 | 材质规则 |
| --- | --- |
| 固定侧栏、固定页顶栏 | 侧栏使用 sidebar 实色，任务页工作区和顶栏使用 surface，其他页顶栏使用 background 实色；不使用背景模糊 |
| 首页媒体舞台 | 清晰源图在前；允许同源图片的 16% 氛围层、20px 模糊，裁切在舞台内，不能覆盖标题、按钮或进度 |
| 信息流圆环进度 | surface-raised 的 78% 半透明底和 6px 背景模糊，文字保持清晰，轨道只绘制圆环 |
| 任务行、设置表单、文案正文 | 实色 `surface`，不使用逐行 blur、封面染色或持续投影 |
| Dialog、菜单、Tooltip、Toast | 实色 `surface-raised`，0.5px 边界和 YAML 浮层阴影；不透出正文干扰阅读 |
| 系统窗口 | 保留原生装饰，不依赖 Windows Mica、macOS vibrancy 或透明窗口；本次不新增平台材质能力 |

应用外壳采用浅色实底，首页媒体舞台与信息流圆环进度保留局部氛围和透明反馈。禁止多层模糊叠加、霓虹光晕和滚动列表滤镜。系统要求降低透明度时关闭媒体氛围，并将圆环进度改为实色底，布局和可读性不变。

常规表面无阴影；菜单与弹窗仅使用各自 `materials.shadow-*`。默认交互即时反馈，任务行背景使用 140ms ease-out，任务右键菜单进入使用 80ms 从 65% 不透明度淡入、退出使用 60ms 淡出，其他必要的颜色或浮层显隐使用 250ms ease-out，不使用弹簧、缩放入场或按钮位移。成功移除行保留既有 220ms 收起生命周期，下方项目自然上移；减少动态效果时立即移除。信息流封面仅在鼠标悬停卡片时以 250ms ease-out 放大至 1.035 倍，仅变换图片且裁切在原容器内，不改变卡片布局；移出卡片后恢复原尺寸。键盘聚焦沿用可见焦点轮廓，不触发缩放；关闭封面预览后正常恢复焦点，不保留放大状态。信息流圆环与圈内数字以 250ms ease-out 向最新真实目标值补间；新事件从当前显示值继续，下降、未知和合并立即切换，不超过已收到的目标。线性进度保持即时。所有 Dialog、AlertDialog 及遮罩以 250ms ease-out 淡入淡出，关闭期间弹窗内容不接收操作，遮罩保留直到退出结束；沿用 Radix 显隐和焦点生命周期，不增加弹簧、缩放或位移。不确定进度辅以文字，减少动态效果时所有上述动效关闭并立即显示真实状态。

### 2.4 `src/style.css` 的唯一映射路径

`src/style.css` 已按本节集中映射，无新增主题依赖。映射顺序固定为 **YAML palette → Radix CSS 原始变量 → 语义变量 → `@theme inline` → shadcn 与业务布局**。

在 `src/style.css` 集中定义本版所需的 `--slate-*`、`--slate-a*`、`--indigo-*`、`--jade-*`、`--amber-*`、`--red-*` 原始值，逐项对应 YAML。以后如改为官方 CSS 导入，须明确依赖变更并删除重复原始值。组件只消费语义变量。不能只写 `var(--indigo-11)` 而未定义或导入。

| 现有 / 目标 CSS 变量 | YAML 来源 |
| --- | --- |
| `--background` / `--foreground` | `colors.background` / `colors.on-surface` |
| `--card` / `--card-foreground` | `colors.surface` / `colors.on-surface` |
| `--popover` / `--popover-foreground` | `colors.surface-raised` / `colors.on-surface` |
| `--primary` / `--primary-foreground` | `colors.primary` / `colors.on-primary` |
| `--primary-text` | `colors.primary-text` |
| `--primary-hover` / `--primary-pressed` | `colors.primary-hover` / `colors.primary-pressed` |
| `--secondary` / `--secondary-foreground` | `colors.surface-control` / `colors.on-surface` |
| `--muted` / `--muted-foreground` | `colors.surface-control` / `colors.on-surface-muted` |
| `--surface-input` / `--surface-hover` | `colors.surface-input` / `colors.surface-hover` |
| `--surface-subtle` / `--surface-pressed` | `colors.surface-subtle` / `colors.surface-pressed` |
| `--accent` / `--accent-foreground` | `colors.surface-selected` / `colors.primary-text` |
| `--border` / `--input` / `--ring` | `colors.border` / `colors.border-input` / `colors.focus-ring` |
| `--border-strong` / `--backdrop` | `colors.border-strong` / `colors.backdrop` |
| `--success` / `--warning` / `--destructive` | `colors.success` / `colors.warning` / `colors.error` |
| `--success-bg` / `--success-text` | `colors.success-bg` / `colors.success-text` |
| `--warning-bg` / `--warning-text` | `colors.warning-bg` / `colors.warning-text` |
| `--error-bg` / `--error-hover` / `--error-text` | 对应 `colors.error-*` |
| `--info-bg` / `--info-text` | 对应 `colors.info-*` |
| `--progress-track` / `--progress-value` | 对应 `colors.progress-*` |
| `--media-placeholder` | `colors.media-placeholder` |
| `--scrollbar-thumb` / `--scrollbar-thumb-hover` | 对应 `colors.scrollbar-*` |
| `--sidebar` / `--sidebar-foreground` | `colors.sidebar` / `colors.on-surface-secondary` |
| `--sidebar-primary` / `--sidebar-primary-foreground` | `colors.primary` / `colors.on-primary` |
| `--sidebar-accent` / `--sidebar-accent-foreground` | `colors.surface-selected` / `colors.primary-text` |
| `--sidebar-border` / `--sidebar-ring` | `colors.border` / `colors.focus-ring` |

在现有 `@theme inline` 中保持 `--color-*` 指向语义变量；新增状态底色、进度和媒体变量按需补齐。`--font-sans` / `--font-mono` 对应 `typography.font-family` / `font-mono`；`--text-xs` / `--text-sm` / `--text-xl` 固定为 12 / 14 / 20px，行高按角色应用；`--button-font-size` 固定 12px。`--radius` 为 8px，`--radius-xs/sm/md/lg/xl` 明确映射为 4/6/8/12/16px，不使用默认推导产生额外圆角。

布局尺寸以同名 CSS 自定义属性映射，如 `--sidebar-width`、`--sidebar-width-icon` 对应 `spacing.sidebar-width`、`sidebar-compact-width`。任务行的 `--task-row-min-height`、`--task-row-active-min-height`、`--task-cover-size`、`--task-status-width`、`--task-action-width` 分别对应 `components.task-row` 的 `minHeight`、`activeMinHeight`、`coverSize`、`statusWidth`、`actionWidth`。`task-list` 的表面、分隔线和悬停色复用语义色，列表圆角为 0，行反馈背景圆角为 8px，`--task-row-gap` 对应 `task-list.rowGap` 的 4px 行间留白；不在页面另建尺寸或颜色常量。

侧栏颜色使用 YAML 定义的 slate-2 / slate-3 混色，阴影使用已定义的 slate alpha 变量。按钮 hover 不再使用 `primary/80`；Badge 不再用透明度猜测状态底色。集中覆盖默认字号、border/ring、圆角和动效，保留官方组件键盘与焦点能力。Radix `asChild` 可替换 `data-slot`，按钮规则按真实元素与 `data-variant` / `data-size` 映射，并为 ToggleGroup 选中态、媒体缩略图、多行内容链接保留明确的样式优先级。滚动箭头使用原生滚动条按钮的 CSS 渐变背景和 Radix slate-11/12；不使用 WebView2 滚动条伪元素无法可靠绘制的 mask，避免遗留旧十六进制色。

## 3. 应用外壳与页面布局

### 3.1 导航和窗口

入口依次为 **下载中心、下载任务、素材库、系统设置、关于软件**，默认进入下载中心，隐藏工作台菜单。各导航项保持同级对齐；已完成记录属于下载任务，作者搜索结果是该页的子视图。下载中心保留原“下载作品”的输入、预览与下载流程。素材库筛选及下载输入在页面切换时分别保留。

侧栏展开宽 192px，收起宽 56px；品牌区高 64px，图标 32px、名称 20px / 600 / 28px（复用 `title-page` 字体规格），当前版本使用 12px 次级文字。导航项高 40px、图标 16px、圆角 8px、项间距 4px；选中使用浅蓝底、蓝色文字和 `aria-current`。收起时保留 Tooltip、可访问名称、当前项及活动任务数，计数不得覆盖图标。主侧栏、设置分类菜单及顶部 Tab 的可用项悬停显示手形光标；非交互连接状态不显示手形。任务计数只包括已提交后的解析、等待、下载，不包含纯预览。

侧栏底部“本地运行 / 连接不可用”来自真实连接状态；不是新操作入口，不展示端口、守护进程或虚构在线指标。保留原生窗口边框与控制按钮，不绘制假标题栏、浏览器地址栏或重复的红黄绿按钮。关闭窗口、托盘恢复、退出与单实例行为沿用现有实现；隐藏窗口可继续任务，退出回收当前操作。

右侧标题区最小高 56px，包含 20px 页标题和必要的上下文操作；不要求每页都有副标题。任务页标题区最小高 64px，显示“下载任务”，右侧保留搜索和清理历史；状态筛选位于标题下方独立一行。

### 3.2 三个设计视口

下表宽度均指应用内容视口，不包含系统原生窗口装饰。

| 规格 | 1440 × 900 | 1280 × 800 | 1024 × 680 |
| --- | --- | --- | --- |
| 侧栏 / 主内容内边距 | 192px / 24px | 192px / 24px | 56px / 16px |
| 内容可用宽度 | 1200px | 1040px | 936px |
| 首页预览 | 两栏 40% / 60%，舞台高 360px | 两栏 40% / 60%，舞台高 320px | 单栏，舞台高 300px，信息在下 |
| 任务行 | 封面 56px，状态 88px，操作 144px | 同左 | 同左；标题单行省略，必要元数据/反馈换行、行高增长 |
| 设置二级栏 / 栏间距 | 184px / 24px | 184px / 24px | 160px / 24px |
| 关于页 | 最大宽 1040px | 使用可用宽度 | 单列纵向滚动 |

窗口宽度 ≤1100px 时收起侧栏、正文内边距降为 16px、首页预览改为单列；宽度 1101–1280px 使用中档舞台；更宽时使用基准舞台。两栏比例按减去内部间距后的剩余宽度计算。空间不足时缩短次要元数据或换行，不缩小字号、隐藏更多菜单、裁切焦点或强制横向滚动。

### 3.3 滚动责任

- 侧栏固定；首页仅主内容纵向滚动，输入和结果按自然文档顺序排列，不把整页锁死在首屏。
- 任务页状态筛选、搜索及平台 Tabs 固定，只滚动媒体列表；列表项不受 Flex 压缩，错误和下载信息可扩高。
- 设置页左侧二级菜单和右侧分类标题固定，只有右侧内容滚动；依赖“重新检测”属于固定分类头部。
- 关于页主内容滚动；弹窗固定标题与操作区，仅正文滚动。代码/表格及图集缩略图可在各自区域横向滚动，不传导为整页横向溢出。
- 滚动条宽/高 6px，轨道透明、滑块圆角；默认 slate-7，hover / 拖动为 slate-12。保留原生上下/左右箭头和滚动方式，箭头同色且按钮背景透明。平台不显示原生箭头时不伪造。
- 任务列表与滚动条间保留 8px，设置、关于、文案正文与滚动条间保留 12px；仅关于页主滚动容器保留右侧 4px 外侧空隙。列表/表单内预留不小于 0.5px 的边界空间，避免 ring 或边线裁切，不叠加不必要的双滚动条。

## 4. 页面 A：下载中心

### 4.1 固定结构与输入

从上到下为页标题、分享链接输入区、解析结果/媒体舞台、下载目录。首页不增加最近记录区或另一套任务列表。

输入区使用 `ShareLinkInput` + shadcn `InputGroupTextarea`，始终三行，高 80px、内边距 8px、14px 字号与 20px 行高；长分享文案在框内滚动，不随输入撑高。右侧纵向放置“粘贴链接”“解析链接”两个 32px 按钮，宽度保持一致；解析成功后后者为次级“重新解析”。下方保留解析引擎 Select：自动、yt-dlp、DTK、tdl，使用简明标签，不使用中点。

“粘贴链接”仅在点击后通过 Tauri 原生命令读取并聚焦输入，不自动解析、不后台监听剪贴板。空剪贴板及失败用 Toast 提醒，保留 Ctrl / ⌘ + V。右键菜单保留全选、剪切、复制、粘贴、清空及原有禁用条件；异步剪贴板操作不得覆盖后输入的新文本。Enter 提交、Shift+Enter 换行，中文输入法组词时 Enter 不提交。

自动模式沿用视频默认 yt-dlp、图集按能力选择 DTK 的现有决策；YouTube 仅 yt-dlp，Telegram 使用 tdl 实验路径。不因网络、凭据或风控错误自动切换引擎。下载进行时仍可解析下一条作品，每次只提交一条。

### 4.2 大封面预览

解析成功后在输入下方直接显示媒体预览，不跳新页面。预览分为 40% 媒体舞台、60% 信息区，中间距 24px、容器内边距 16px；舞台圆角 16px，外层不再叠加装饰性阴影卡。

媒体舞台根据视口高 360 / 320 / 300px。清晰封面位于视觉中心，`object-fit: contain`，使用真实自然比例，横版、竖版、方形都不拉伸或强裁成 9:16。可用同源弱模糊层填充剩余空间；没有封面则使用等尺寸中性占位，不显示可播放三角形误导用户。点击清晰封面打开大图预览，不启动视频播放。

信息区依次显示：作品标题（14px / 600，最多两行）、作者/频道资料、平台/实际引擎/解析状态、时长或图片数量、真实清晰度与大小、规格选择、原声信息及必要警告、进度/错误、下载操作。可换行但保持组间距 12–16px；不让格式标签挤压封面。源内容和实际数值优先，作品缺标题时使用真实可用链接或中性占位，不伪造作者和时长。

底部动作右对齐：“后台下载”为主按钮，“立即下载”为次级描边按钮（视频与图集统一命名），以及当前状态允许的取消/重试；下载目录为页面底部 12px 路径与“修改设置”入口，长路径省略并可查看完整值。

### 4.3 规格、图集和解析缓存

- 视频规格复用 `api.chooseFormat()` 和现有单选 ToggleGroup，默认最高分辨率；两栏预览中规格区按空间使用两列，窄窗改一列，最大高 196px 后内部滚动。选项最小高 56px，14px 尺寸，12px 容器/编码/帧率/码率与大小；不使用 11px。选择通过标记、浅蓝底和边界共同表达。
- 只显示真实可下载规格及稳定 ID。显示实际尺寸、容器、编码、已知大小；估算明确标“约”，未知为“大小未知”。YouTube 需合并时注明 FFmpeg，无损合并不转码。下载重新提取同一规格，规格失效明确失败，不自动降级。重启和重试保留选择。
- 图集复用 `AlbumGallery`，使用同一舞台尺寸、可操作缩略图条、当前序号和每张“原图/高清”来源、已知尺寸；不将整套图集都标原图。图片原图优先、高清次选，仅在原图候选缺失或原图 HTTP 404/410 时使用高清；网络/身份/风控错误不静默降级，保留原格式，不转码或截图。
- `cache_hit` 只显示低权重“解析缓存”，不代表已下载或地址仍有效。缓存来自现有 SQLite 白名单元数据/预览；下载仍重新提取并校验作品身份。正常重启可恢复完整 ready 预览，遗留 `parsing` / `queued` / `downloading` 标为 `failed` / `INTERRUPTED`，不自动续传。
- 作者头像、昵称、ID 和频道语义见第 7 节；Cookie 自动读取警告与封面错误不能覆盖真正的任务状态。

### 4.4 首页状态与提交行为

| 状态 | 预览与操作 |
| --- | --- |
| 无输入/无结果 | 小型媒体占位、标题“粘贴链接开始解析”；保留正式平台标签，Telegram 按实验能力表述 |
| `parsing` | 原结果区显示骨架/“正在获取作品信息”，按钮“正在解析…”，禁用链接输入及其右键编辑操作、粘贴和重复解析，结束后恢复输入；提供既有取消；不伪造百分比 |
| `ready` | 显示“已解析，可下载”、完整预览与规格，主动作变为下载 |
| `queued` | 保留当前预览并显示真实等待位置，无速度和假进度；可取消 |
| `downloading` | 预览内展示与任务页一致的进度，规格不再可改；可取消 |
| `failed` / `canceled` | 保留可用作品信息与持续原因，按第 6 节提供重试/重新解析 |
| `TDL_CHANNEL_ID_REQUIRED` 且有 Telegram 链接 | 显示等待服务说明与“加入待下载队列”；沿用既有延迟解析路径 |

“后台下载”在 Rust 接受任务后才提示“已加入下载任务”，立即清空本次输入与预览、聚焦输入，下载或等待仍在任务页继续。失败或取消重复下载确认则保留预览。“立即下载”沿用原下载队列和提交流程并保留首页过程，当前作品完成后清空对应预览与尚未被用户改动的输入。后台其他作品完成、迟到事件或初始历史加载均不得清空新输入或重新占用首页。

重复下载按当前作品身份、类型和规格判断：同平台/作品的同规格成功记录，或规格未知的历史成功记录，且文件仍存在时提示“该规格已下载过”；图集和 Telegram 按其原始内容身份判断，不引入虚构规格。单纯失败、取消、预览或文件缺失记录不触发此提醒。这是对旧文档“同作品无条件提醒”的纠正，依据 `src/lib/download-identity.ts` 当前契约。

提示在首次解析命中及再次下载提交校验时沿用现有流程；展示原路径，提供“暂不下载”“重新下载”。暂不下载保留预览；确认只授权当前规格，切换规格后重新判断；确认后保存新文件，不覆盖原文件。

## 5. 页面 B：下载任务

### 5.1 统计、筛选与本地搜索

页标题右侧保留“清理历史”及搜索框；下方第一层为全部、下载中、等待中、已完成、失败状态筛选，状态导航高 40px，标签高 32px、左右内边距 12px、标签间距 8px，以底部下划线突出当前状态；第二层为全部、抖音、TikTok、YouTube、Telegram 平台 Tabs，高 52px，标签紧凑左对齐。状态标签旁的计数由当前平台与搜索匹配的真实任务派生，不受当前状态选择影响，排除已移除的退出快照，不新增统计卡片。状态、平台与关键词取交集；下载中包含已提交的解析任务，已取消仍可在全部查看。状态筛选不影响批量清理原有的平台范围与确认语义，作者结果子视图不套用状态筛选。搜索标题或作者资料，只查询本地记录，不访问平台搜索。搜索框有内容时，右侧显示“清空搜索”图标按钮；点击清空关键词及作者限定并将焦点返回输入框，保留当前平台和状态筛选；内容为空时隐藏按钮。

常规列表只包含 `download_requested` 的记录，纯解析/ready 预览不进入，也不计入侧栏活动数。等待任务按真实 FIFO 位置排在前，其余按创建时间倒序；同时间保持稳定顺序，完成不自动消失。筛选、搜索、退出动画均不得破坏任务身份或已移除记录屏蔽。

点击可识别身份的作者昵称进入本地作者结果子视图，侧栏仍高亮下载任务；有 ID 按平台+ID 精确匹配，旧记录按既有账号名规则回退，不混入不同 ID 的同名作者。该视图可包含已有的解析预览，且不显示批量清理按钮。手动编辑搜索框恢复普通记录搜索；返回下载任务退出作者筛选。

### 5.2 连续媒体列表与 80px 常规行

平台 Tab 行最右侧提供图标按钮切换列表与信息流，默认列表；按钮显示下一种展示方式的图标、Tooltip 和可访问名称。切换只改变排版并将列表滚回顶部，保留平台、状态、搜索、任务顺序和当前选择；展示方式保存在本机，跨页面与应用重启恢复上次选择；首次使用或无有效偏好时默认列表。启动时同步恢复，避免先出现列表再切换；仅保存展示方式，不保存筛选或滚动位置。空状态仍保留切换入口。列表和信息流默认渲染前 30 条，距滚动面板底部 500px 时提前追加 10 条，追加期间显示旋转加载图标；已加载项目保留 DOM 身份与滚动位置。切换平台、状态、关键词或作者结果时从前 30 条重新展示，计数和批量操作仍针对完整筛选结果。

信息流采用自适应等宽多列网格，最小列宽 240px、列间距 16px、行间距 20px。卡片不加外边框或阴影，封面为 3:4，居中等比例铺满；右下角仅显示真实时长或图集图片数量，未知时不显示。“已完成”徽章固定在封面左上角，唯一的播放图标位于右上角；已完成且文件存在的图集以右上角图集图标打开既有图集浏览弹窗，底部不再重复展示“查看图集”；文件缺失记录不提供播放。下载时封面中央展示 64px 圆环和圈内百分比，沿用真实字节进度并最多显示 99%；大小未知时显示“—”，合并时显示“合并”及真实阶段文案，不伪造百分比。圆环使用原有 shadcn/Radix Progress 的平滑圆形样式和半透明底，速度、字节数及剩余时间保留在封面下方信息区，下载期间封面使用实色底、淡化灰度图片并禁用点击，圆环和遥测保持清晰可用；下载中卡片不渲染底部状态及操作区，取消下载收纳到任务右键菜单，沿用原有确认与取消流程。封面下依次为最多两行标题、作者与来源、原有下载信息；下载及合并期间隐藏清晰度、图片数量和文件大小等规格行，由下载进度替换该行，规格与进度区统一保留 40px 两行空间、换行间距 4px，移除进度顶部额外留白及完成卡片的空操作区，避免普通下载、合并与完成卡片高度跳变；下载结束后恢复规格信息，列表模式保持原样；其他状态与操作在底部对齐，窄卡片或长操作可换行。下载进度、排队位置及文件缺失持续可见；失败原因置于封面顶部的实色错误提示内，长错误可滚动查看，不在下方重复显示。异常内容允许卡片增高。卡片封面与其他区域共用任务右键菜单，合并保存封面、复制封面与原有任务操作；封面不可用或保存/复制进行中时禁用对应封面操作，下载中封面仍支持右键取消下载；列表模式保留独立封面菜单，详情弹窗保留封面预览菜单。封面、详情、作者搜索、播放和删除沿用列表行为，不加载第二套任务数据，也不在卡片内嵌播放器。仅 Tab 面板滚动，切换不增加动画。

平台筛选不显示标签下划线或整栏底边框，仅以浅蓝背景标识选中项。列表空状态同样无外边框、圆角和阴影，与上方筛选区域自然衔接；仅保留空状态信息，不显示“去下载作品”按钮。

平台筛选与列表直接融入连续白色 `surface` 工作区，不再套带外边框和圆角的内层卡片；每行无独立边框、阴影、外间距或行间分隔线；通过 8px 圆角的悬停、选中背景定位任务。悬停只使用 `surface-subtle` 的轻微背景差异，不改变尺寸；点击行或聚焦行内控件后，当前行使用 `surface-selected` 浅蓝底。菜单展开时对应行同样保持活动底色；键盘焦点沿用可见轮廓。单击选择仅用于视觉定位，不触发下载、播放或删除。双击已完成且本地文件可用的视频任务行信息或空白区域，在应用内弹窗播放本地视频；忙碌中、图集及其他任务状态不触发。行内按钮、作者链接、封面及详情弹窗保留各自交互，不冒泡触发行双击播放。列表仍由平台 Tab 面板独立滚动，容器不额外创建第二个滚动区。

每行是横向四列布局：**56 × 56px 封面容器 → 弹性信息区 → 88px 状态区 → 144px 操作区**，列间距 12px、水平内边距 16px、垂直内边距 8px。常规已完成行采用 80px 最小高度，落在 76–84px 的常规密度范围；等待任务最小高 84px，状态列显示队列位置；下载中最小高 120px，等待说明、失败原因、封面错误、必要元数据换行或特殊长文字动作可自然增高，不能裁切反馈来维持固定高度。

任务列表封面容器始终等尺寸，圆角 8px、0.5px 边界；视频和图集首图使用 `object-fit: cover` 居中等比例铺满，允许裁切边缘，不拉伸或平铺重复图片。首页与大图预览仍完整显示原始比例。缺封面、加载失败和文件缺失不改变列宽，不显示虚假缩略图。点击封面查看大图，行内不嵌入播放器。

常规信息区为两层：单行标题，以及作者、来源、清晰度/图片数量、大小、下载时间组成的元信息，空间不足时自然换行，两层间距 6px。异常反馈可以额外扩高。标题 14px / 600、行高 20px，超长省略，保留全文 title；标题为普通文本，单击仅选中任务，不再打开独立详情。点击封面（支持 Enter/Space）打开封面与详情合并的实色 Dialog，左侧封面按自然比例完整展示，右侧展示完整标题、作者与作者 ID、来源、作品 ID、任务 ID、创建/完成时间、完整文件路径和媒体规格；内容可选择复制、长路径换行，右侧正文独立滚动，关闭返回封面按钮。弹窗宽 960px（最大视口减 48px）、高 600px（最大 85dvh），左右等宽、间距 24px；三个设计视口均保留左右布局。左侧封面区域复用下载作品页的同源弱模糊背景（16% 不透明度、20px 模糊及相同底色），前景封面保持清晰和自然比例，减少透明度偏好下隐藏模糊层。封面缺失时左侧显示占位，右侧详情仍可访问。不以逐行展示 ID 或完整路径挤占浏览空间。下载完成时间紧跟文件大小，使用已有本地时间格式；未完成或没有完成时间的记录不显示时间。

任务行不重复展示作者头像，昵称保留本地作者搜索行为；来源使用中性文字，12px，始终紧邻作者，不放在行尾。作者长名称占元信息行最多 30%，省略且可查看全文，完整 ID 放在详情中。清晰度、图片数量、大小与文件缺失提示同属第二信息层，不添加多余图标；文件缺失仍以红色文字和提示图标持续显示。已完成大小使用实际文件字节，清晰度按真实短边显示如“1080P”，悬停给完整宽高。详情内时间格式如“2026年10月16日 20:48”，未知字段显示“—”或明确未知。

标题缺失或仅含空白时，按真实来源、媒体类型和作品标识展示，例如“Telegram 视频 1001234567890_12345”；仅有 Telegram 链接时展示来源与原链接，没有作品信息时使用任务标识。不编造作品名称，不将已完成作品显示成“单条视频解析”。

状态和操作分别位于最后两列，都相对整行垂直居中；状态不放在右上角，操作不沉到右下角。常规已完成视频只显示 32px 无边框播放图标，保留“播放视频”可访问名称与 Tooltip；图集显示“查看图集”。文案等次要操作位于整行右键菜单，不再显示末尾更多图标。直达动作不超过两个；作者结果 ready 预览、Telegram 延迟解析等特殊长文字动作可在操作列内换行，整组仍居中，必要时行高增长。

### 5.3 下载进度

列表与下载作品页的进度位于信息区元数据下方，不跨过封面或与右侧操作混排；信息流使用上述封面圆环。线性轨道高 4px，`progress-track` / `progress-value`；下方显示真实百分比、已接收/总字节、速度、可用 ETA，图集另显示已完成/总张数。遥测为 12px / 500，允许窄窗换行。

总量优先实际 `total_bytes`，再用明确标“约”的 `total_bytes_estimate`；没有总量使用不确定进度和已接收量，速度/ETA 不存在时不估造。下载阶段百分比最多 99%，只有 Rust 校验、提交文件及记录成功后才显示已完成。

`merging=true` 时显示“正在合并音视频…”和不确定阶段反馈，不将传输完成当作业务完成，也不伪造合并速度。取消期间显示“正在取消…”，等待后端回收结果后才显示已取消；合并、图集均占一个原有下载槽位。

### 5.4 行尾操作与更多菜单

下表决定默认入口位置；所有入口仍复用原有回调、忙碌/禁用判断和错误传播。仅移动入口，不扩大动作权限。

| 任务条件 | 直达高频操作 | 更多菜单中的次要操作 |
| --- | --- | --- |
| 已提交的解析、等待或下载中 | 取消，`canceling` 时禁用并显示过程 | 有原链接时复制原链接 |
| 已完成视频且文件存在 | 播放 | 提取/查看文案、打开文件夹、复制原链接、从列表移除、删除本地视频 |
| 已完成图集且目录存在 | 查看图集 | 打开文件夹、复制原链接、从列表移除、删除本地图集 |
| 已完成视频但文件缺失 | 无文件动作 | 已有文案时查看文案、有链接时复制原链接、从列表移除 |
| 已完成图集但目录缺失 | 无文件动作 | 有链接时复制原链接、从列表移除 |
| 失败/已取消且有作品身份，或已提交的 Telegram 链接 | 重试 | 有链接时复制原链接、从列表移除 |
| 失败/已取消且无法重试 | 重新解析 | 有链接时复制原链接、从列表移除 |
| 作者结果中的 `ready` 预览 | 下载视频/图集、取消 | 有链接时复制原链接；不提供历史移除或文件删除 |
| 尚未提交且命中 Telegram 延迟解析条件 | 加入待下载队列 | 有链接时复制原链接；其他动作遵循原条件 |

文案入口位于更多菜单的打开文件夹之后（无文件夹入口时排在首项），图标/名称区分“提取文案”“正在提取文案”“查看文案”；运行中点击查看同一操作进度，不重复启动。图集与未完成记录不显示文案入口。文件缺失但有旧结果仍可查看。任务列表播放为 32px ghost 图标按钮，没有文字与边框，保留可访问名称；下载作品页保持原有按钮及更多入口。查看图集、取消、重试使用清晰文字。

任务列表的更多操作由 shadcn `ContextMenu` 提供，整行右键打开，不提供末尾更多图标。任务行可聚焦，Shift+F10 或菜单键可打开同一菜单；封面区域优先保留已有保存/复制封面菜单。菜单宽 236px、条目最小高 36px、内边距 8px、圆角 10px，使用独立的轻柔 `shadow-task-menu` 阴影，在指针附近显示，由 Radix 自动避让窗口边缘。进入为 80ms ease-out 从 65% 不透明度淡入（无位移），退出为 60ms ease-out 淡出，退出期间不拦截点击；减少动态效果时立即显隐。菜单内部使用图标和文字，不另套 Tooltip。顺序为文件夹、文案、复制链接、复制本地地址、分隔、记录移除、删除文件，按原条件省略不适用项。保留方向键、Enter、Escape；关闭返回当前任务行，任务移除后返回相邻任务或列表，菜单转文案弹窗不抢焦点。下载作品页沿用原有 DropdownMenu 入口。

“从列表移除”与“删除本地视频/图集”名称必须区分；删除文件使用红色文字/图标及与其他菜单项一致的浅蓝聚焦背景，不能只用垃圾桶而无名称。不将危险操作放到封面点击或整行点击上。可用动作临时忙碌时禁用，非适用动作按现有条件隐藏；复制原链接须有规范链接或 Telegram 链接。复制本地地址仅供有本地路径的已完成视频使用，复制完整绝对路径并反馈结果；图集不显示该项。播放只禁用当前任务的播放按钮并显示加载反馈，避免其他任务按钮随全局忙碌状态闪烁；当前视频弹窗打开期间不重复启动同一任务；内置播放器加载反馈位于弹窗内，不切换全局忙碌状态。封面保存/复制仍保留在封面右键菜单和预览内，不因行尾收纳而丢失。

播放优先使用 HTML `<video>` 和浏览器原生控制栏，在现有 Dialog 中展示，最大宽 960px、媒体区域高 `min(56dvh, 480px)`，视频按原比例完整显示；支持播放/暂停、拖动进度、音量及浏览器提供的全屏控制。初始读取及缓冲显示加载反馈，关闭时立即暂停并解除媒体源，退出动画仍沿用 Dialog。内置播放器按任务及本地路径、完成时间在本机保存播放位置，每约 5 秒节流保存，暂停、拖动完成、关闭、页面隐藏或退出页面时立即保存；再次打开或重启后在元数据加载后继续上次位置，播完或回到起点清除进度。路径或完成时间变化、无效或超出实际时长的记录不用于恢复。移除任务或删除本地文件时同时清理该任务进度，关闭后的清理不得将其写回；外部播放器进度不接管。保存失败仅提示一次且不影响当前播放。关闭返回播放按钮或任务行，触发器消失时返回列表。仅已完成、文件存在的视频可播放；图集沿用图集浏览。

前端仅按任务 ID 调用 `prepare_video_playback`，Rust 复用播放路径校验，仅动态授权该普通视频文件给 Tauri asset 协议，静态文件 scope 为空。媒体使用原生 Range 能力，不通过 IPC/Blob 整体传送；不内置解码器、不修改或转码原文件。浏览器解码失败、只识别音频轨或无法建立媒体源时，自动调用原有系统播放器入口，每次播放会话最多自动切换一次并提示结果；自动播放策略限制由原生播放控件供用户继续操作，不因未自动播放而打开外部应用。弹窗也提供“使用系统播放器”入口。外部启动失败保留错误和手动重试；关闭、任务移除或切换会话后，迟到媒体结果不得重开或启动外部播放。

### 5.5 历史与文件管理

| 操作 | 执行条件与结果 | 确认规则 |
| --- | --- | --- |
| 重试 | 失败/取消且身份可用；复用任务 ID、创建时间、实际 resolver 和所选规格，使用最新配置重新解析并从头下载，加入 FIFO 队尾 | 沿用原流程，不增加确认 |
| 重新解析 | 无可重试身份；回首页聚焦链接输入 | 不擅自创建下载 |
| 取消 | 解析/ready/等待/下载的既有入口；只取消当前操作并清理暂存，保留记录 | 既有 AlertDialog：“返回 / 确认取消” |
| 从列表移除 | 已完成/失败/取消，且不再被工作进程占用；删除任务及其记录，保留本地媒体 | 直接执行，无二次确认 |
| 删除本地视频/图集 | 已完成且文件/目录存在，后端确认可操作；删除任务所属媒体并同步删除任务与数据库下载记录 | 直接执行，无二次确认 |
| 清理历史 | 当前平台 Tab 内可清理的已完成/失败/取消记录，搜索不改变范围；全部 Tab 才覆盖全部平台 | 既有批量确认，显示平台范围及保留文件的结果 |

文件缺失是独立维度：保留“已完成”历史状态，在元数据行用红色小号文字及图标提示“文件已移动或删除”，无边框；不显示播放、打开文件夹和本地删除，也不新增未实现的“重新定位/关联文件”。保留复制链接、移除及已保存文案。

单条操作处理中禁用重复点击；失败保留记录并持续显示原因。批量清理保留进行中、排队、纯预览及仍被进程占用的记录，整批成功或回滚；只清理记录、提交证据及无剩余引用的缓存字节，不删除本地文件。成功后提示实际条数，失败保留弹窗供重试。

删除成功才播放收起动画；批量可见项目同时退场，失败不动画。关闭菜单/弹窗通常返回触发器；触发器随成功删除消失时，聚焦相邻可用行操作或列表容器并 `preventScroll`，不主动跳搜索框。批量清理后清理按钮不可用则回列表，避免滚动跳动。

空列表：“还没有下载记录”，提供“去下载作品”；当前平台为空：“该平台暂无下载任务”；搜索无结果：“没有匹配的下载任务”。说明只在有帮助时保留一句，不添加演示计数。

## 6. 状态、错误与反馈

### 6.1 下载状态映射

`Task.status` 仍只有下列七种；`canceling`、`merging`、`file_missing`、`cover_error`、`cache_hit` 是附加维度，不扩充后端状态枚举。

| 数据 | 可见文字 | 视觉和边界 |
| --- | --- | --- |
| `parsing` | 解析中 | info 徽标，纯解析在首页；已提交重试解析可在任务列表 |
| `ready` | 已解析，可下载 | info 徽标和真实预览；不计为下载任务 |
| `queued` | 等待中 / 等待解析 | warning 徽标，真实队列位置，无虚构速度 |
| `downloading` | 下载中 | info 徽标及进度；合并阶段改为明确阶段文案 |
| `completed` | 已完成 | success 徽标，以最终提交为准 |
| `failed` | 失败 | error 徽标、图标和持续原因，不将整行染红 |
| `canceled` | 已取消 | 中性徽标，可按条件重试 |
| `canceling=true` | 正在取消… | warning 徽标与禁用取消，不提前改终态 |
| `file_missing=true` | 文件已移动或删除 | 与历史状态并列，收回失效文件动作 |
| `cover_error` | 封面未保存 / 预览不可用 | 稳定占位与低权重提示，不把视频任务改失败 |
| `cache_hit=true` | 解析缓存 | 中性标签，不冒充已下载 |

徽标高 24px、12px / 500、图标加文字，不能只用色点区分。平台与引擎标签使用中性样式，错误与缺失资源不抢占真实下载状态。

### 6.2 错误表达

沿用 `errorText()` 的稳定映射。任务错误位于任务信息区，用图标和 12px 语义色文字，无额外边框；必要时换行，不能被省略到只剩 Toast。表单错误紧邻字段、带 `aria-invalid` 与关联说明；全局连接错误位于页顶。重试失败不清空上次有效信息。

网络/代理失败、平台拒绝访问、平台要求验证、资源不可访问、规格失效、目录写入失败、引擎/依赖缺失须保留不同原因和可行补救。不把所有 403 或请求失败解释为 Cookie 过期，不把公开作品解析成功称为登录验证成功；无法区分的原因使用通用失败，不推断作品已删除。

使用真实、简短、面向桌面用户的文案；“解析成功”“已加入下载任务”“已完成”是三个不同阶段。未知分辨率、大小、速度、ETA 显示未知，不凭封面、分辨率或计时器推算。

### 6.3 Toast、加载和键盘

空分享链接提交时，以 Toast 提示“请输入视频分享链接”，并聚焦链接输入框，不新增页内错误提示。

Toast 复用 Sonner，顶部居中、实色浅表面、0.5px 边界，标题 14px、辅助内容 12px；状态图标和语义色表达成功/错误/警告/信息，默认和加载使用中性样式。关闭按钮位于内部右上角，正文预留空间，中文可访问名称“关闭通知”。操作反馈仅提示一次；恢复已有记录不重放成功/失败，Toast 不替代页面持续错误。

输入、控件、行操作均覆盖默认、hover、pressed/selected、focus-visible、disabled、busy。忙碌状态使用真实 Spinner 与动作名称，禁用重复提交；未检测不显示“可用”或“缺失”。Skeleton 不包含伪造作者/大小/进度。

键盘顺序为导航、页内输入/筛选、结果和操作；图标按钮有中文名称。Tabs 支持方向键，菜单支持方向键/Enter/Space/Escape、焦点可见。Dialog 限定焦点范围，关闭返回合理入口；不可因收纳菜单丢失焦点恢复。保留系统复制粘贴、文本选择与触控板滚动，尊重减少动态效果。

## 7. 媒体、封面与作者资料

### 7.1 封面读取、预览、保存和复制

首页与任务行共用 `VideoCover` 的行为；视频封面来自平台，Telegram 新下载原始视频通过现有 FFmpeg 从前 30 秒抽取非近黑 JPEG，失败保留占位。抖音以原有封面为基准，只有实际尺寸更大、静态且画面高度一致的候选才替换，保留原始图片字节；候选不可用或无法确认时沿用基准，详见 vendor/UPSTREAM.md。缺少封面不影响视频成功，不补处理历史记录。图集以实际图片预览，沿用 `GalleryImage` 的受控读取。

新下载视频封面字节只存现有 SQLite 缓存，不自动放入视频目录；旧同目录封面保持原样。优先读取已有本地/数据库字节；前端使用现有受控 API 与 Blob URL，共享进行中的读取和已加载图片，页面切换与筛选不重复读取。缓存按任务及图片来源标识，来源或缓存时间变化后重新加载；普通进度修订不清空封面。保留最多 128 张、64 MiB 未使用的近期图片，仍显示的图片不提前回收；淘汰或任务移除并结束退出动画后释放 Blob URL，失败读取不永久缓存。不要为媒体舞台另建远程图片代理或下载逻辑。

下载作品页点击可用封面打开“封面预览”Dialog，最大 90vw × 90dvh，自然比例，清晰图可键盘聚焦。下载任务页点击封面则打开左封面、右详情的合并弹窗，规格见任务列表章节。任务缩略图与大图的右键菜单保留“保存封面/另存为…”和“复制封面”；无图片、加载失败或保存/复制中按原条件禁用，图像可用性变化时关闭下载作品页的失效预览；任务详情弹窗保持可用，封面不可用时展示占位。保留系统保存对话框、取消不报成功及失败反馈。

保存封面是用户显式导出图片，与自动缓存不同；复制使用已有像素读取及原生剪贴板接口，不改变下载作品。图集原图依旧按整套独立目录保存、全套校验与不覆盖提交后才完成。移除最后一个同作品任务/预览时，按现有同事务引用规则清空封面/头像 BLOB，保留解析文字元数据与别名；不删除旧旁路文件。

### 7.2 作者与频道

使用已有 `AuthorInfo`、shadcn Avatar 与 `author_profile`：显示真实昵称/账号名和缓存头像，缺头像回退首字符或中性占位；旧记录可用 `author`，不自动联网补全。首页头像 32px 并保留 ID；任务行优先显示昵称，昵称为空时显示作者/频道 ID；两者均缺失的旧记录沿用账号名或 author 信息。完整 ID 仍可在任务详情查看，其他 compact 场景仍使用 24px。作者长名称省略并提供全文，详情中的 ID 使用 12px，不与标题争主位。

抖音、TikTok、YouTube 保留各自已确认的作者 ID/handle 语义，昵称不统一添加 `@`。Telegram 显示频道 ID与已知公开用户名，缺名称时为“Telegram 频道”，不猜个人发帖者或频道头像。昵称只有在可匹配身份时作为本地作者搜索入口，详见 [作者资料契约](docs/author-profile.md)；头像失败不影响作品解析/下载。

## 8. 页面 C：系统设置

### 8.1 分类结构与外观

保持左右布局：左侧二级导航 184px，窄窗 160px，栏间距 24px；右侧为当前分类。左栏不套卡片、不增加“设置分类”装饰标题，使用 0.5px 右侧分隔。分类为下载设置、平台账号、运行依赖；导航项高 40px、14px / 500，选中浅蓝、普通 hover 为中性底，不用彩色左线。

右侧分类标题 14px / 600，必要说明 12px；表单分组背景 `surface`、圆角 12px、内边距 16px、字段间距 12px。标题/操作与字段边缘对齐；辅助说明不重复标签。输入、Select 高 32px，Cookie/UA 使用多行区域，密码/长路径不撑宽布局。1024 × 680 仍保留左右布局，右侧内部滚动。

### 8.2 下载设置

下载目录为单行只读字段和“选择文件夹”，沿用系统目录选择与即时保存。并发使用 Select 1 / 2 / 3，默认 2，即时保存；提高上限立即补位，降低不打断已运行任务。队列为 FIFO，每次提交单个作品，设置改变不恢复重启后中断的任务。

下方提供“文件命名规则”卡片，包含规则输入、恢复默认、快捷插入变量、示例实时预览和完整路径。默认 `{platform}/{user_name}-{content}`，支持 `{platform}`、`{user_name}`、`{link_id}`、`{content}`、`{current_time}`、`{quality}`；当前时间为下载开始时本地时间 `YYYYMMDD_HHmmss`，清晰度取真实短边，未知时标明未知。作者缺失使用“未知作者”，描述缺失使用作品 ID。当前解析协议没有发布日期且不支持批量列表，不展示相应变量。

变量在光标或选区处插入，保留焦点；规则通过“保存命名规则”或 Enter 保存，失败保留草稿。恢复默认先恢复草稿，保存后生效。`/` 分隔相对路径，最多 4 段（3 层子目录），拒绝空目录、上级目录和反斜杠；仅替换完整且受支持的变量，未知变量、未闭合或多余的花括号按普通文本保留，不报变量错误。快捷插入变量按钮悬停显示手形；实时预览按平台 jade、作者 amber、作品 ID indigo、描述 violet、当前时间 cyan、清晰度 orange 的第 3 阶背景和第 12 阶文字显示，同一变量保持同色；固定文字、目录分隔符及扩展名不着色。Rust 同时返回最终路径与带变量来源的片段，清理及截断后仍逐字一致。已保存规则的成功预览在应用会话内保留，切换页面后立即复用，不重复请求或显示加载占位；只缓存当前已保存规则，未保存草稿和失败结果不替换缓存，保存成功后更新。下载目录前缀随设置直接更新；预览的实际命名仍由 Rust 计算。预览仍保留 150ms 输入防抖，更新期间保留上一帧内容和路径，不切换加载占位；标记计算中并禁止保存，失败时清空无效结果并显示错误，迟到响应不能覆盖新结果。预览和下载复用 Rust 命名逻辑；扩展名按下载实际容器自动追加，图集只命名独占目录，内部图片名称保持原样。非法文件名字符替换为下划线，处理 Windows 保留名称及过长文本；已有重名文件或目录添加 ` (1)` 等序号，保留原有不覆盖提交。规则持久化到 SQLite，并在提交下载时保存到操作快照，仅影响新提交的下载及重试；不重命名已有文件。

### 8.3 平台账号

右侧顶部为下划线 Tabs：抖音、TikTok、YouTube、Telegram，每次只显示一个平台；隐藏面板不进入焦点顺序，切换分类/平台保留未保存输入。抖音/TikTok/YouTube 每个平台一张配置表单，Cookie、代理、YouTube HTTP 分块、User-Agent 按需分组；Telegram 使用独立账号流程。

- Cookie 经专用读取接口在多行输入中回显，初次展示可见，支持显示/隐藏和编辑，沿用当前账号编辑器行为；旧文档默认掩码的描述不再适用。仅在账号编辑器显示，不进入普通设置摘要、事件或日志。沿用“使用系统默认浏览器 Cookie（保存后清除手动值）”，手动值优先；留空只尝试系统当前 HTTPS 默认浏览器，不扫描其他浏览器、不回写自动值。读取失败持续显示原因并禁用编辑/清除，保存其他字段用 `cookie=null` 保留原值。
- 网络代理保留直连、自定义 HTTP/SOCKS5 与原有配置读取/保存；仅自定义显示地址。代理密码仍经系统凭据存储，未修改时传 `proxy=null` 保留。读取失败持续显示，不用空值覆盖已有配置。系统 TUN 可用直连，不新增 TUN 模式开关。
- YouTube 保留“继承上游默认 / 关闭分块 / 自定义”HTTP 分块设置；自定义为 1–1024 MiB 整数，非法时提示并禁止保存。继承与关闭不是同一值；只影响新任务下载，不更改视频规格或音轨。
- User-Agent 保持与 Cookie 来源匹配的浏览器标识。当前平台底部“保存平台配置”同时提交该平台修改，不提交其他平台，未改动/无效/忙碌时禁用。保存成功及失败均使用 Toast 提醒并标明平台，不在表单内嵌入保存结果提示；只影响新任务，进行中任务沿用配置快照。
- 保留公开解析不等于登录身份有效的必要提示；不新增未实现的“测试 Cookie”按钮，也不以公开作品成功模拟认证检测。

### 8.4 运行依赖

固定分类头显示“重新检测”与一条必要依赖策略；本次运行自动检测一次，结果、错误和进行中状态跨页面保留，切换不重复检测。卡片顺序为 **FFmpeg → JavaScript 运行时 → tdl → Fun-ASR-Nano**，下方是下载位置与内置引擎说明。

每项卡片使用相同标题、状态徽标、版本/路径和来源位置。独立的下载、修复、取消与进度都放在该卡片内容区，不使用单独灰底页脚。可用、缺失/版本不足、检测中、未检测必须区分，不能把未知当作缺失。

| 依赖 | 保留能力与交互 |
| --- | --- |
| FFmpeg / FFprobe | 实际版本、路径、系统/应用专属来源；缺失时“下载 FFmpeg”，包含 FFprobe，已可用不重复安装 |
| JavaScript | Node.js 22+ 或 Deno 2.3+ 满足其一；缺失时“下载 Deno”，已有符合要求的运行时不重复下载 |
| tdl | 用户自行安装，只检测 `tdl version`；显示实际版本/路径，不绑定版本或宣称登录有效 |
| Fun-ASR-Nano | 一张语音模型卡，下载/重新下载修复；Windows x64 按实际可用能力选择 CPU / CUDA，macOS 保留 CPU；不展示虚假可用后端 |

模型选择只改变待安装选项，不立即切换运行后端；安装验证成功才保存，失败/取消保留原安装和后端。模型大小随选择显示实际返回值，未知明确未知；CPU 约 1.2 GiB、CUDA 约 1.0 GiB 仅为理解量级，首次附加程序/组件和实际传输大小以数据为准。CUDA 错误提供现有修复或选择 CPU 的明确路径，不静默改后端。

tdl 路径使用单行输入及右侧无文字文件夹按钮、Tooltip 和中文名称；选择系统单文件后自动保存并立即检测，同路径也检测，取消不改配置或提示成功。手动输入/清空按 Enter 保存检测；保存失败保留旧配置，检测失败保留已保存路径并持续报错。过程中禁用选择/输入，不恢复底部“检测 tdl / 保存路径”按钮。自动查找范围沿用 PATH、GOBIN、GOPATH/bin、Go 配置及常见安装目录。

依赖下载保留真实字节、总量、校验、解压、执行验证和重新检测，支持取消与错误重试；仅通过验证才成功。下载进入应用数据目录 runtime，不改系统 PATH、不覆盖系统工具，不使用平台 Cookie 或平台代理；不支持自动下载的系统明确说明。Python、yt-dlp、DTK 随软件分发，通过应用更新升级，不提供单独热更新。

### 8.5 Telegram 实验模式与登录

HTTP 实验开关、风险确认、登录/重新登录与取消登录统一在平台账号 → Telegram；运行依赖只提供 tdl 检测和路径。两处保存保留另一处配置，检测/保存/登录忙碌时沿用互斥条件，活动 Telegram 任务需先取消。

默认不启用。启用沿用 AlertDialog，必须清楚说明已知 tdl HTTP 服务全接口监听、无访问令牌、无消息白名单，loopback 与随机端口不构成隔离；这段必要风险信息不能因减少说明文字而删除。tdl 检测不代表已登录或实际操作可用，详见 [tdl 集成说明](docs/tdl-integration.md)。

用户点击登录后用实色 Dialog 呈现连接、二维码、两步验证、验证中与失败；二维码周围保留安静留白，不加玻璃、滤镜或模糊。通过已保存路径调用官方 QR 登录，使用映取新建独立会话，不读取、导入、覆盖已有 Telegram Desktop/tdl 会话。2FA 只在官方要求时出现，提交、关闭和结束清空密码，短时内存传递，不写配置、数据库、日志或普通事件。

最长等待 5 分钟；Escape/关闭会取消本次登录，与更新/文案弹窗的关闭继续后台处理不同。失败/取消保留原会话配置并持续显示原因，成功只表示独立会话已配置，实际权限以解析/下载为准。Telegram 预览只提供真实原始文件规格，未知时长/尺寸/编码不猜测。

## 9. 页面 D：关于软件与更新

### 9.1 关于页

沿用统一外壳，内容最大宽 1040px 居中，最小视口内纵向滚动。品牌区为本地应用图标、20px “映取”、构建注入的当前版本和一句简介；品牌区底部展示抖音、TikTok、YouTube、Telegram 中性徽章；Telegram 提示文本标注实验模式，不将实验能力作为正式承诺。

按品牌/版本操作、已有能力、项目支持、开源致谢顺序组织。能力和支持区在宽屏可两列、窄窗单列；常规分组圆角 12px、内边距 16px、间距 16px，无大阴影与装饰背景。保留复制版本信息、手动检查更新、现有下载进度/安装包入口。

项目源码、问题反馈、Apache-2.0 许可证均指向本仓库，由现有 opener 在系统浏览器打开。致谢保留 yt-dlp、DTK、Tauri、React、shadcn/ui、Lucide 的已核实仓库和键盘入口；折叠区展示真实组件版本与本机工具要求，完整来源/许可指向 `vendor/UPSTREAM.md`，不把摘要称为完整传递依赖列表。

页面离线可读，不加载远程贡献者头像、赞助二维码或虚构更新日志。应用版本来自构建，不能把旧硬编码引擎文案当作当前安装版本事实。

### 9.2 更新流程与弹窗

“检查更新”仅在用户点击后由现有 Rust 命令请求本仓库 GitHub Releases 最新正式版本并按 SemVer 比较；检查中只在按钮显示 Spinner 和“正在检查…”，禁用重复请求。无更新使用 Toast，失败在关于页持续展示并保留发行页入口，发现更新打开 Dialog。

更新 Dialog 宽 640px、最大高 85dvh，标题/底部操作固定，正文滚动。显示新版本、当前版本、真实发布日期、GitHub Markdown 更新说明、适用安装包名称和真实大小。说明正文 14px，各级标题统一 14px / 600，通过间距区分；支持列表、加粗、代码、引用、表格及只读任务列表，不执行原始 HTML，图片只显示替代文字；HTTP/HTTPS 链接走系统浏览器，其余链接为文字，失败原因在弹窗内可见。

自动匹配 Windows x64、macOS Intel、Apple Silicon 的安装包，缺适用包或 SHA-256 则禁用自动下载并保留“前往发行页面”。下载、取消、校验、完成及失败沿用 `UpdateDownload`，真实字节/总量/百分比按可得数据展示；取消需等后台停止并清理后才显示已取消。失败可重新下载。

“后台下载”、Escape 和遮罩关闭弹窗均继续当前更新下载，焦点回检查按钮，关于页可查看进度；切页再返回恢复现有操作，不重复启动。下载完通过大小和 SHA-256 校验后才提供“打开安装包 / 重新下载”；打开时再次校验，文件缺失或被修改提示重新下载。

更新存于应用数据目录 updates，使用系统网络/系统代理，不用平台凭据。启动时及新安装包校验并保存成功后自动清理缓存，按语义版本只保留最高版本的一份安装包；同版本优先保留当前刚下载且可打开的文件，否则保留较新的文件。只清理应用生成的 UUID-Yingqu 安装包，不处理其他文件、子目录或符号链接；未完成的新下载不淘汰旧完整包。被占用等原因无法删除的旧包保留，后续清理时再尝试，并记录诊断。没有自动检查、静默安装、自动退出/重启；用户下载并打开安装包后继续安装。退出取消未完成下载，重启后重新检查下载，不承诺断点续传。

## 10. 弹窗、菜单与视频文案

### 10.1 通用组件规格

所有文本、数字、密码和多行输入统一关闭浏览器自动填充，由共享 `Input`、`Textarea` 设置 `autocomplete="off"`，组合输入控件沿用该行为。

| 组件 | 尺寸与视觉 |
| --- | --- |
| Button | 标准/次级/ghost 高 32px；紧凑附属按钮 28px；文字 12px，标准水平内边距 12px，图标 16px |
| 多行内容链接 / 图集缩略图 | 项目支持链接为自动高度、最小 56px、padding 8px；图集缩略图 40 × 48px，选中使用浅蓝边界和 CircleCheck 标记，均不套标准按钮固定高度 |
| Input / Select | 标准高 32px、文字 14px、圆角 8px、0.5px 输入边界；搜索可用 28px 紧凑规格 |
| Textarea | 14px 自然换行；首页固定三行 80px，Cookie 初始四行且沿用可调整高度，不套单行高度 |
| Card / 媒体列表 | 分组圆角 12px、实色和 0.5px 边界；任务列表融入白色工作区，不另套外边框，内部行无分隔线及外边框，相邻行背景间留白 4px（随移除动画收起），以 8px 圆角悬停/选中背景提供反馈，任务行 padding 按 YAML |
| Dropdown / ContextMenu | 通用最小宽 192px、条目最小高 32px；任务更多菜单宽 236px、条目 36px、padding 8px、圆角 10px。14px 文字，实色浅表面及轻柔任务菜单阴影 |
| Tooltip | 12px，实色浅表面、0.5px 边界、小圆角，内容简短，不替代必需的持续错误 |
| 通用 Dialog / AlertDialog | 默认宽 480px、最大视口减 32px、最大高 85dvh、padding 20px、圆角 16px、实底和弹窗阴影 |
| 专用 Dialog | 视频文案 720px、更新 640px、图集 680px；各自最大高见 YAML，宽度仍受视口减 32px 限制 |
| Alert | 通常内联图标和文字；必要风险分组可浅语义底、0.5px 边界，避免整块饱和色 |

弹窗标题 14px / 600，说明 12px / 400，内容正文 14px；标题与关闭按钮留出独立空间。上下操作区域使用相同实色，以间距组织，不给每个弹窗增加灰色大页脚。长正文使用 `min-height: 0` 与单一滚动区，正文块不被 Flex 压缩；底部按钮在 1024 × 680 仍可见，必要时换行，不覆盖文字。Dialog 保留焦点约束与中文关闭名称。

普通菜单无遮罩，打开/关闭不滚动页面。ContextMenu 保留输入编辑和封面操作的现有作用域，不为所有页面启用浏览器默认菜单。菜单键盘关闭返回触发器；菜单转弹窗时先结束菜单焦点生命周期，再将焦点交给弹窗。

### 10.2 关闭、确认与后台处理

| 对象 | 关闭或操作语义 |
| --- | --- |
| 单条移除/本地删除 | 直接执行，不新增确认 |
| 当前任务取消 | 保留取消确认，仅操作该任务 |
| 重复下载 | 保留当前规格确认，不覆盖已有文件 |
| 批量清理历史 | 保留范围确认及失败后重试，忙碌时防止重复提交 |
| Telegram 启用 | 保留风险确认 |
| Telegram 登录 Dialog | Escape / 关闭取消登录并清空密码 |
| 视频文案 Dialog | Escape / 关闭只收起，继续提取 |
| 更新下载 Dialog | Escape / 遮罩 / 后台下载只收起，继续下载 |
| 封面/图集 Dialog | 关闭结束查看，回到可用的原触发器 |

不要因为统一弹窗外观而统一这些不同的操作语义。

### 10.3 单视频文案

保留 `VideoTranscriptDialog` 与现有转录客户端：仅已完成视频可开始，图集不支持；每次只提取一个视频，转录不改变下载状态。已有文案在原视频缺失后仍可查看、复制、导出；没有文件时不能新提取或重新提取。

标题“视频文案”，宽 720px、最大视口减 32px、最大高 85dvh。标题及底部固定，只读正文独立滚动；保留换行、全文选择和连续长文本换行，提取时间排在正文之后，不被文本或按钮覆盖。展示该份结果实际模型和 CPU/CUDA 后端；旧结果保留 SenseVoiceSmall，重新成功后才更新 Fun-ASR-Nano，不用当前安装模型冒充历史结果来源。

依赖不足时提供手动检测/下载、模型版本选择和缺失 FFmpeg 的独立下载；安装完成仍需用户点击“开始提取”。提取中显示真实音频提取、语音分段、模型识别阶段及不确定进度，可取消；其他视频正在提取时提供查看该操作入口，不开启第二个。关闭窗口继续处理，再开恢复状态。

成功提供复制全文、系统保存对话框导出 TXT、重新提取；只导出已保存正文，不混入时间和提示。失败、取消、中断、无语音及源文件变化保留各自状态，存在上次成功结果时继续显示并说明来源，不能用空白结果覆盖旧文案。操作错误持续显示；关闭后恢复入口焦点，入口已消失则回到合理列表/导航位置。完整边界见 [Fun-ASR-Nano 集成方案](docs/fun-asr-nano.md)。

## 11. 后续前端实施验收

### 11.1 本版文档验收

- YAML 可解析、无重复键，所有 `{路径}` 引用存在且无循环；颜色追溯到 Radix，材质/阴影不夹带另一套调色板。
- 正文与 YAML 的字号、行高、按钮/输入、侧栏、媒体舞台、任务行、进度、菜单和弹窗尺寸一致；特殊尺寸有明确作用域。
- 四页及其子状态完整，任务动作矩阵覆盖现有入口、显示/禁用条件和执行副作用；没有旧任务卡要求与新视觉并存。
- Markdown 标题、表格、链接及本地路径有效；运行 `git diff --check`。本次规范交付不等于界面、打包或平台验收完成。

### 11.2 实施后的视觉与行为验收

后续修改前端时，在 1440 × 900、1280 × 800、1024 × 680 检查外壳、缩放、长标题/路径、横竖封面、图集、所有任务状态和大段文案；Windows 125%/150% 缩放检查 0.5px 边线、焦点与文字清晰度。保留 Windows/macOS 原生装饰、键盘、托盘和减少动态效果支持；关闭媒体氛围后仍可用。

逐项验证解析引擎/规格选择、剪贴板、封面预览/保存/复制、作者本地搜索、队列/取消/重试、重复提醒、后台下载、文件缺失、移除/删除/批量清理、文案提取/导出、平台草稿/凭据隔离、运行依赖、Telegram 实验登录和关于/更新。截图或 fixture 使用明确隔离的示例，生产数据必须来自真实接口。

## 12. 轻量素材库页面契约

本节是 [知识库技术方案](docs/workbench-knowledge-base-plan.md) 对应的 UI 实施契约。WB-113～WB-118 已实现导航、知识库列表、分类标签、资产详情、文件播放、文案笔记、基础工作台及导出备份恢复界面。阶段原生验收和 MCP 仍按后续任务实施，进度与验证边界见 [任务清单](docs/workbench-knowledge-base-tasks.md) 和 [实施记录](docs/workbench-knowledge-base-validation.md)。

### 12.1 导航与布局

复用现有应用外壳和 YAML token，不新增颜色、文字尺寸或装饰性标签。导航顺序见第 3.1 节；现有链接输入、解析预览及下载能力收归下载中心，不因导航调整丢失当前任务。

知识库主区域由分类筛选、列表和按需进入的资产详情组成。1440 × 900 与 1280 × 800 可同时展示分类栏和主区域；1024 × 680 保留导航与可折叠分类入口，避免再强塞第三列。主内容具有明确滚动容器，不让播放器、文案和底部保存按钮互相覆盖。

### 12.2 列表、分类和标签

列表顶部使用“全部视频”“待分类”“待整理”同级 Tab 切换范围。左栏为一级分类列表，分类标题旁的加号在列表内展开新建输入框，Enter 添加、Escape 取消，不弹窗。分类管理仅负责改名和删除。列表保留关键词、分类、标签、平台及文件状态筛选；移除已移出、导入视频和刷新图标入口。研究专题、选题草稿和视频比较退出默认导航。分类、标签及批量操作复用现有菜单、Dialog、Checkbox、Select 和可访问焦点行为。

多分类按 OR、标签按 AND 组合；切换范围清空所选分类，切换筛选后回到首页并清空勾选。分类筛选下“全部视频”Tab 仍为当前范围；在“待分类”中选择具体分类时切回“全部视频”。每页 20 条，通过“加载更多”追加；全选仅覆盖已加载视频。顶部 Tab 及筛选固定，只有列表滚动；分类与标签过长时在各自区域滚动。1024 × 680 通过“分类”按钮展开分类栏，范围 Tab 始终可见并支持方向键切换。应用变更事件、返回页面或窗口重新聚焦会重新读取当前查询，不保留失效分类 / 标签筛选。

一个视频可勾选多个一级分类和标签；分类仅匹配直接关联，历史层级不扩展筛选。新建分类使用空 parent_id；桌面与 MCP 不接受新建或调整上级。旧分类 ID、关联和备份信息保留，旧层级在界面中平铺。历史删除约束继续保护旧关系；删除分类或标签不删除资产和文件。新建、重命名及删除都有结果反馈，错误保留在操作处。

批量“分类与标签”以全部 / 部分 / 未关联表示当前状态，仅提交明确勾选过的关系。每项关系使用核心原子批量命令，不将多个关系伪装为同一事务；部分成功后显示已保存项数并保留剩余修改，重试只提交尚未完成项。分类标签名称最多 128 个 Unicode 字符，不提供上级分类选项。

无资料时显示“暂无视频”；筛选无结果显示“未找到匹配视频”及清除筛选入口。未知作者、时长、规格保持未知，不由文件名推测。文件缺失仍展示标题、封面和文案入口，播放入口说明缺失并提供重新关联。

### 12.3 资产详情与正文编辑

单击列表标题或聚焦后按 Enter 进入详情，返回列表保留搜索及筛选并恢复焦点。左侧为视频播放器，分类和标签放在视频下方，可通过“修改”调整；文件状态、路径、规格和来源信息也归入左侧。右侧仅提供“原始文案 / AI 总结”两个页签。不展示文件版本、文案版本、笔记摘要、历史片段或关键帧入口。

原始文案来自 AI 或本地模型提取，可人工修改并保存。界面默认读取最近保存的原文或人工修改，搜索命中文案时打开命中的内容。保留现有不可变识别证据、人工文稿来源链和并发保存校验，不增加数据库迁移，不因界面精简删除历史资料。新提取完成不覆盖当前输入，提供“读取最新内容”。提取、重新提取、取消和真实进度仍在原始文案页签中。

AI 总结记录对文案或视频的写作手法、表达手法和画面分镜手法的分析，可编辑 Markdown 并预览。复用现有笔记正文和修订接口；默认读取最近更新的 AI 或手法总结，其他历史笔记仍可读取编辑，不冒充 AI 总结，多条内容使用简短选择器，不显示修订号。结构化旧分析继续保存原记录，纯 Markdown 修改另存自由文本总结。界面不提供复制和导出。

切换页签、内容、资产或离开编辑页有未保存修改时，沿用“保存后离开 / 放弃修改 / 继续编辑”。保存失败保留输入；冲突后重新读取，保留本次输入并可恢复编辑；写入成功而回读失败时只重读，不重复提交。普通刷新和新转录不替换当前编辑内容。Markdown 不执行 HTML，不加载远程图片。

1440 × 900 和 1280 × 800 使用媒体与正文等宽双列，1024 × 680 改为纵向。详情独立滚动，长路径和 Markdown 代码、表格在自身区域换行或滚动，不扩大窗口。用户点击播放后校验资产与文件关系，使用原生媒体控制栏和等比显示；离开详情释放媒体。播放失败、文件缺失和变化持续显示真实状态；重新关联仍复用原生选择器和核心探测，保留原有资料及稳定 ID。

### 12.4 移除、删除与恢复

| 操作文案 | 可观察结果 |
| --- | --- |
| 移出分类 | 只移除当前分类关系，视频仍可从全部视频找到 |
| 移除下载记录 / 清理历史 | 按下载中心既有交互执行，保留知识库文案、笔记、封面及视频 |
| 移出知识库 | 从默认列表隐藏，操作完成后提供反馈，可在“已移出”恢复 |
| 删除本地视频 | 使用现有明确文件删除动作；知识库显示文件缺失，保留资料 |
| 重新关联文件 | 原生选择器选择文件并经核心检查，成功后恢复播放入口，旧文案仍保留原始来源信息 |

第一阶段不提供永久删除知识资料。已提交视频但知识资料保存失败时，下载状态仍为完成；现有任务错误区域持续展示“视频已下载，知识资料保存失败，请保留下载记录并重新启动应用”。修复前移除历史明确失败，避免删除唯一可恢复资料；不额外弹出一次性提示替代持续错误。

### 12.5 工作台和备份入口

工作台按最近访问、待分类、待整理及正在处理的操作提供直接入口。待整理以“尚未创建整理稿且没有笔记”的在库资产计算；访问记录持久化，失败操作有可达的原始详情。无事项时显示简单空态，不用演示内容、收藏统计或 AI 猜测填满页面。 每个资料分区显示最多六项与真实总数；待分类、待整理可进入完整筛选列表。1440 × 900 和 1280 × 800 使用两列，1024 × 680 改为单列，沿用主内容滚动区。运行中操作与需要处理的失败操作分开；按任务 ID 或资产 / 文件 ID 跳转，刷新失败保留上次结果并提供重试。

单篇导出位于资产详情的文稿 / 笔记操作区。设置“知识库”分组提供备份和恢复。默认选择“仅资料备份”，选择“包含视频”时展示实际范围和可获得的体量。恢复明确要求空知识库，已有资料时禁止覆盖并说明原因；取消文件或目录选择不开始操作。完成结果显示实际保存位置、恢复数量或失败原因，不显示尚未实现的增量合并选项。

MCP 设置沿用导航与操作区，默认关闭；显示真实的读取、关闭、运行、开启中、关闭中和异常状态。开启后提供只读本机 Streamable HTTP 连接地址及“复制地址”，不展示命令启动、适配程序路径或 TOML/JSON 格式切换。默认地址为 http://127.0.0.1:17653/mcp，异常或关闭时不可复制。保留“允许 AI 写入”开关，服务关闭时禁用并清除授权；展示仅允许读取或已允许写入，写入范围为笔记、分类标签、转录和画面采样。所有操作均有等待、成功与持续失败反馈。保留本机访问及模型数据使用说明，沿用现有 token、12/14px 字号、0.5px 分隔与键盘焦点。具体契约见 [MCP 设置](docs/mcp-settings.md)和[本机 HTTP](docs/mcp-streamable-http.md)。

前端验证依据现有 `package.json` 执行 `npm run build` 并执行 `git diff --check`；浏览器预览与 Tauri 真实运行分开记录，Windows / macOS Intel / Apple Silicon 分开标注 PASS、FAIL、NOT_RUN 或 UNAVAILABLE。源码构建不能替代真实平台、视觉、在线下载或全新设备安装验收。本次已实现本规范的前端重构，最终交付与验证范围见 [UI 调整汇总](docs/task-ui-final-validation.md)，各阶段记录保留当时的运行证据。


备份恢复使用独立滚动区及现有 Field / ToggleGroup / Button / Progress，不新增视觉 token。选择备份位置后由核心新建唯一子目录；恢复分别选择备份目录与恢复文件保存位置，任一步取消都不开始操作。页面展示真实文件处理计数 / 字节、阶段、终态结果与位置；准备阶段使用不确定进度，不能用模拟百分比。进入最终保存阶段后禁用取消，已接受的取消须先于提交生效。切页后可继续处理，返回设置或点击工作台操作入口恢复当前状态；关闭应用等待取消和实际 worker 回收。

“备份说明”按需展开，区分可迁移的资料逻辑包与可能含设置 / 凭据、仅供本机版本回退的升级前数据库备份。恢复缺媒体是可用结果，提示可在知识库重新关联；非空库、活动任务、选择失败、损坏包及连接失败保持各自错误或禁用原因，并提供可执行的重新读取或重试。

### 12.8 轻量素材库入口

主导航依次为下载中心、下载任务、素材库、系统设置、关于软件。原工作台不再单独展示；选题、专题、比较和结构化拆解退出默认导航，已有服务与历史资料保留。

下载成功仍登记资产与文件，新资产默认未收录。完成视频在卡片模式中使用心形图标收藏，未收藏为空心，收藏后为红色实心；再次点击或按 Space 可取消。收藏使用当前任务的独立等待状态，不禁用其他卡片，不改变卡片选中态或重载封面。列表模式沿用文字操作，任务菜单可打开详情或移出。重复下载保持已有收藏状态。列表默认只展示已收录素材，展示封面、标题、作者、平台、分类、标签和文案状态，支持关键词、分类、标签、平台筛选及批量分类标签操作。内部手动导入服务继续保留，默认素材库不展示导入与已移出入口；重新收藏仍可从下载任务完成。

搜索命中的文案或笔记可直接打开相应内容。转录、备份与恢复进度保留在对应操作处；关键帧不在素材详情提供入口，已有数据与 MCP 服务保留；未保存保护和文件生命周期沿用既有契约。1440 × 900、1280 × 800 使用双列详情，1024 × 680 使用单列。
