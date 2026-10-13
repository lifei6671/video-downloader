# 抖音 / TikTok 轻量桌面下载器技术方案

日期：2026-10-13。当前决策：**后端解析与下载统一采用 yt-dlp，废弃原 dtk / wreq 方案及 Rust 媒体 HTTP 下载链路。**

状态：M1 代码已统一为 yt-dlp，旧引擎与生产依赖已退役；离线测试与双平台源码/打包下载通过；抖音详情请求使用用户批准的最小签名补丁，用户已批准进入 M2；全新无 Python Windows 环境验收移至 M3，仍待验证。详见[当前 M1 验证](m1-validation.md)；[独立试验](ytdlp-validation.md)与[旧 M1 历史](m1-legacy-validation.md)分别保留。

## 1. 产品范围与决策

目标技术栈：**Tauri 2 + React + TypeScript + Rust 应用核心 + Python/yt-dlp sidecar + SQLite**。界面名称“映取”，视觉与交互以 `DESIGN.md` 为准。

用户已决定由 yt-dlp 取代原 `Evil0ctal/Douyin_TikTok_Download_API` 依赖。yt-dlp 同时承担平台提取、详情请求、媒体选择与下载；Rust 管理任务和文件最终提交，不再接手签名媒体 URL 发起第二条 HTTP 下载链路。首版两个平台都走 yt-dlp，不保留旧引擎回退。抖音在新引擎下仍须重新验收。

| 能力 | 首版行为 |
| --- | --- |
| 输入 | 每次单个作品，支持长链、短链或包含一条链接的分享文案；多链接拒绝 |
| 预览 | 标题、作者、封面、时长；可选字段缺失不阻止下载 |
| 平台设置 | 抖音、TikTok 各自独立 Cookie、User-Agent、可选 HTTP/SOCKS5 代理 |
| 下载 | 实际下载字节、速度；总大小未知时显示不确定进度 |
| 并发 | 同时下载默认 2，可配置 1–3；超出容量进入 FIFO 等待队列 |
| 任务操作 | 取消、手动重试、打开文件夹、移除记录 |
| 删除文件 | 明确的本地文件删除操作，必须二次确认；移除记录不删除文件 |
| 持久化 | 重启恢复任务记录与已完成列表，不恢复网络下载 |
| 分发 | Windows 11 x64 优先，随后 macOS Intel / Apple Silicon；用户无需安装 Python |

不做作者批量采集、评论、平台内容搜索、图集、直播、身份池、自动游客身份、内置登录浏览器、浏览器 Cookie 自动读取、云同步、开放 API 或 MCP。不提供暂停、断点续传、退出后后台下载、自动重试、转码或分轨合并。历史列表搜索不等于平台内容搜索。

只选择同时包含音视频的单文件、直接 HTTP(S) 下载格式；无音轨作品可保存其单文件视频。仅有 HLS/DASH、分离音视频或需要外部后处理的作品返回 `UNSUPPORTED_CONTENT`。不为扩大 yt-dlp 站点覆盖而引入 FFmpeg、JS runtime、插件或自动更新服务。FFmpeg 仅用于开发验收解码，不随产品运行依赖分发。

## 2. 当前证据与依赖取舍

### 2.1 已验证与未验证

- 固定基线：`yt-dlp==2026.8.19`，运行时版本为 `2026.08.19`；Python 开发基线 3.12。
- TikTok 作品 `7676209499589446945`：现有 Cookie、Edge 150 UA，SOCKS5 与 HTTP 代理均下载成功；687,985 bytes，1024×576，约 18.85 秒，音视频完整解码通过。
- 以上为迁移前独立试验，脚本现已退役；当前 sidecar 与 Rust 探针结果见 M1 验证记录，双平台和安装包未全部通过。
- 新引擎抖音下载、双平台长短链组合、正式进程取消、重启恢复、打包与 macOS 均须重新验证。旧引擎抖音成功不可替代新引擎验收。

### 2.2 目标依赖

| 项目 | 决策 |
| --- | --- |
| `yt-dlp==2026.8.19` | 唯一平台解析与媒体下载引擎；后续升级显式锁定版本 |
| Python 标准库 | JSONL、CookieJar、输入校验及轻量协议映射 |
| PyInstaller | 开发/构建依赖，打包项目自己的 Python bridge 与 yt-dlp |
| pytest 等现有测试工具 | 按实际保留的测试使用，不混入运行依赖 |
| Rust `tokio`、`serde`、`serde_json`、`anyhow` | 延续现有进程、异步 I/O、消息与错误处理用途 |
| Tauri、SQLite 与系统凭据访问 | 按桌面阶段的实际工程引入，不把拟定选型写成已安装 |

该 yt-dlp 基线的包元数据没有必选第三方 Python 依赖；`default`、`curl-cffi` 等为可选 extras。首轮沿用已成功的最小包，不默认安装全部 extras。某平台确需额外网络依赖时，必须由复现证据决定并同步锁定、许可证和打包验收。[官方依赖声明](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/pyproject.toml)

以下依赖与资产已从 M1 实现退役，相关引用与失效测试同步删除：

| 旧资产 | 迁移处理 |
| --- | --- |
| `vendor/dtk/`、旧 manifest、初始化补丁 | 删除，取消固定旧上游提交和 vendoring 工作流 |
| `pydantic`、`structlog`、`wreq`、`httpx` 及其仅由旧引擎引入的传递依赖 | 从 Python 输入清单移除并重新生成运行/开发锁文件 |
| 旧 `engine.py` 的签名、parser、指纹模拟与 transport 编排 | 替换为 yt-dlp 适配；不保留兼容 wrapper |
| Rust `reqwest` 及旧下载专属依赖/feature | 删除旧媒体 HTTP 调用后移除；保留仍被文件或测试使用的依赖 |
| `src-tauri/src/downloader.rs` 的 HTTP 流式下载实现 | 移除；任务调度、文件校验与最终提交归 Rust 核心 |
| `src-tauri/media-domains.json` 的 dtk 生成来源 | 取消旧生成链；必要的双平台目标约束依据 yt-dlp 实际请求与验收重新确认 |
| 旧 X-Bogus/A-Bogus golden、dtk parser replay 与专属 fixtures | 删除已失效测试；用 yt-dlp 适配契约与离线下载 fixture 替代 |
| `vendor/UPSTREAM.md` 中旧引擎说明 | 实际删除旧 vendor 时退役，改为记录 yt-dlp 版本、许可证与打包依赖 |

迁移只更新项目虚拟环境，不卸载用户全局环境或删除已有下载文件。`DESIGN.md`、`docs/ui` 与真实历史验收记录不因引擎迁移而改写。

## 3. 架构与职责

```mermaid
flowchart TD
    UI[React / TypeScript] -->|Tauri commands| Core[Rust 应用核心]
    Core -->|任务快照与进度| UI
    Core <-->|stdin / stdout JSON Lines| Engine[Python bridge + yt-dlp]
    Engine --> Platform[抖音 / TikTok 详情与短链]
    Engine --> CDN[媒体 CDN]
    Engine --> Stage[任务独占暂存文件]
    Core -->|校验与不覆盖提交| Stage
    Core --> DB[(SQLite)]
    Core --> Secrets[系统安全存储]
```

| 层 | 职责 | 边界 |
| --- | --- | --- |
| React | 输入、预览、设置、任务与历史 | 不访问详情接口、不启动进程、不提交媒体 URL 或任意文件路径 |
| Rust | 任务唯一状态源、队列、配置快照、进程生命周期、取消、进度归并、文件最终提交、数据库与凭据 | 不计算签名、不下载媒体、不另建平台提取器 |
| Python bridge | 单链接识别、配置选择、调用 yt-dlp、结果及错误映射、脱敏事件 | 不维护业务队列、不写 SQLite、不读取浏览器或系统凭据 |
| yt-dlp | 短链展开、平台提取、详情与媒体网络请求、流式保存暂存视频 | 仅启用两平台单视频提取器，不开放任意 CLI 参数或插件 |
| SQLite | 任务、路径、元数据、设置与 Cookie | 用户批准 Cookie 直接保存；不保存代理密码或带 token 的详情/媒体 URL |

解析与下载均使用同一任务创建时的两平台配置快照；bridge 统一识别平台并只使用对应配置。设置修改只影响新任务。凭据经 stdin 进入专属子进程内存，平台 Cookie 限定到对应 HTTPS CookieJar 域，不能作为全局 Cookie header 发送给所有 CDN。

## 4. yt-dlp 适配方式

采用 Python `YoutubeDL` 嵌入方式，由项目自己的 bridge 暴露固定协议；不解析面向人的 CLI 日志，不把裸 yt-dlp 命令行开放给前端。使用内存 CookieJar、自定义 logger 与 `progress_hooks`。[官方嵌入说明](https://github.com/yt-dlp/yt-dlp#embedding-yt-dlp)

- 预览阶段调用 `extract_info(..., download=False)`，只返回白名单展示字段、作品身份与规范链接；不返回原始 info dict、媒体 URL 或 headers。
- 用户点击下载、取得槽位后，通过规范作品链接重新提取并下载。下载前校验平台与作品 ID 和预览一致；不复用预览时的过期媒体地址，也不增加本地媒体 URL TTL 缓存。
- 仅选择符合第 1 节限制的格式，不使用会触发分轨合并的默认格式选择。提取器、格式选择与禁止后处理要在真实 yt-dlp 层验证，不能只断言配置字典。
- 禁用下载续传、自动网络重试、playlist、额外字幕落盘、info JSON、缓存、插件及用户全局配置加载；保存一个视频文件及最高可用分辨率封面。Cookie 不导出临时 Netscape 文件。
- `progress_hooks` 只映射下载字节、已知总字节与速度。估计总大小不作为准确百分比的分母，缺失字段为 null。hook 的 `finished` 不等于业务完成。
- 官方 logger 的自由文本默认不直接输出；错误映射稳定错误码，无法可靠区分原因时保留通用失败，不猜测 Cookie 失效或风控。
- 不承诺 User-Agent 等于 TLS 浏览器指纹。废弃旧 wreq 的 `windows-chrome` 指纹预设和匹配规则；首版保存 UA，未验证的 impersonation 能力不进入配置契约。现有设计素材若出现指纹相关内容，实施时记录此技术边界，不改写素材掩盖差异。

## 5. 协议与进程生命周期

以下为当前 M1 **协议 v2**，Rust 与 sidecar 已共同升级，不兼容旧 v1，不增加旧协议 fallback。

### 5.1 进程模型

每次解析或下载操作使用一个独占 sidecar 进程，一次操作结束即退出。预览解析最多同时执行一个；下载进程数量由 Rust 下载槽位限制为 1–3（默认 2）。这是取消与并发隔离边界，不维护常驻多请求引擎或额外进程池。

Rust 保存句柄并持续读取 stdout/stderr。UTF-8 JSONL，一行一条消息并及时 flush；stdout 仅协议，stderr 仅脱敏诊断。当前 M1 启动上限 15 秒、单次解析含启动总上限 80 秒；读取协议无进展上限 65 秒，终端后退出上限 5 秒。下载不以固定短总时限截断长视频，yt-dlp 使用 socket timeout；超时会终止并回收操作进程。

取消、解析超时、协议损坏或应用退出：Rust 终止对应进程，等待退出后再清理任务暂存文件；不能仅标记 UI。Windows 验证父进程异常退出时子进程回收，macOS 分别验证对应机制。stdin EOF 需能结束正在处理操作的 sidecar，而非仅在同步 yt-dlp 调用返回后才检测；实现时用进程回收机制与 EOF 监测验证，不因选用同步 API 放弃取消契约。关闭网络任务后才释放槽位。

### 5.2 消息契约

| 消息 | 字段与约定 |
| --- | --- |
| `ready` | `protocol_version=2`、`engine_version`、`yt_dlp_version`；Rust 校验与分发版本一致 |
| `resolve_video` 请求 | `id`、单条 `input`、两平台配置快照；凭据只经 stdin |
| `download_video` 请求 | `id`、规范作品链接、预期 platform/video_id、原任务配置快照、Rust 创建的专属 staging 目录 |
| `resolved` | 同一 `id`、platform、video_id、canonical_url、title、author、cover_url、duration_ms；可选字段可空 |
| `progress` | 同一 `id`、downloaded_bytes、total_bytes、speed_bps；不包含上游原始 dict |
| `download_finished` | 同一 `id`、平台/作品身份、staging 内相对文件名、实际字节数与容器；仅表示引擎文件写入结束 |
| `error` | 同一 `id`、稳定 code、脱敏 message、stage；可可靠获得时附 HTTP 状态 |

请求和响应均包含 `protocol_version`。每个操作最多一个终端结果：解析以 `resolved` 结束、下载以 `download_finished` 结束，或以 `error` 结束。成功还必须伴随正常进程退出；EOF、超限、错 ID、错版本、重复终端消息、非零退出或缺失结果都明确失败。单条消息上限 1 MiB。

`download_finished` 之后 Rust 校验路径确实位于所分配暂存目录，拒绝目录、符号链接/重解析跳转及非预期文件，再执行第 7 节文件提交。这个消息不直接映射为 UI 的 completed。

错误码：`INVALID_INPUT`、`UNSUPPORTED_PLATFORM`、`UNSUPPORTED_CONTENT`、`CONFIG_INVALID`、`ACCESS_DENIED`、`NOT_FOUND`、`RISK_CONTROL`、`NETWORK_ERROR`、`UPSTREAM_CHANGED`、`ENGINE_UNAVAILABLE`、`ENGINE_PROTOCOL_ERROR`、`TIMEOUT`、`IO_ERROR`、`INTERRUPTED`。只有可验证证据才细分登录、删除或风控；403 不自动解释为过期或 Cookie 无效。

## 6. UI 命令与状态同步

流程：设置 → 粘贴单链接 → 解析预览 → 点击下载 → 排队/下载 → 文件与记录提交 → 已完成。沿用首页、下载任务、已完成和设置四个视图。

| Tauri 命令 | 行为 |
| --- | --- |
| `parse_video` | 输入分享文本，创建 task_id，异步解析并发出快照 |
| `start_download` | 仅接收 task_id，使用 Rust 保存的作品身份和任务配置排队 |
| `cancel_task` | 取消等待或终止对应子进程，资源释放后进入终态 |
| `retry_task` | 新建任务，重新解析与从零下载；原失败记录保留；无有效链接时要求重输 |
| `list_tasks` | 状态筛选、分页，返回权威快照 |
| `get_settings` / `save_settings` | 非敏感设置和凭据配置状态；替换/清除 Cookie 必须明确操作 |
| `select_download_directory` | 系统目录选择与可用性校验 |
| `reveal_download` | 从记录定位文件，缺失则提示 |
| `remove_history` | 仅移除终态记录 |
| `delete_local_file` | 对应记录的明确二次确认后删除本地视频；保留记录并标记文件已删除，失败保留现状 |

事件 `task_updated` 返回完整展示快照：task_id、递增 revision、状态、预览、字节/速度、脱敏错误及完成路径。约 250 ms 合并进度，终态立即通知。前端先监听再读快照，按 revision 合并；重挂载重新查询。未完成最高显示 99%，文件与数据库提交后才显示 100%。

Cookie 在设置页通过 get_profile_cookie 专用命令回显多行输入，支持显示/隐藏。空输入保存即清空；未完成读取时禁止保存。公开作品解析成功只证明该请求成功，不证明登录态有效。未知清晰度、大小与速度明确显示未知；失败原因持续可见。

## 7. 状态、文件与持久化

### 7.1 任务状态

`parsing → ready → queued → downloading → completed`。解析/下载可转 failed，解析、预览、排队和下载均可取消。终态不被迟到事件覆盖。

Rust 是唯一状态所有者。降低并发限制不打断已开始的下载，仅约束后续调度。队列获得槽位后由 yt-dlp 重新提取作品。取消与完成串行裁决：完成已提交则保持 completed；取消先接受则不得再发布成功。重试创建新任务，使用新任务创建时的设置快照。

### 7.2 文件提交

1. Rust 在选定下载目录下排他创建任务独占暂存目录，确保与最终文件同卷。yt-dlp 只能写入该目录，使用固定安全模板和自身 `.part`；不以标题作为未经处理的路径。
2. yt-dlp 流式下载，关闭续传与覆盖；下载结束完成其暂存目录内的重命名并关闭文件。失败或取消后由 Rust 在进程退出后清理本任务明确拥有的残留文件，不递归清理用户下载目录。
3. Rust 收到成功结果并确认进程正常退出后，核对作品身份、路径、非空文件、允许的容器与基本文件特征、实际大小；已知总大小必须匹配。HTML/JSON 错误页、截断或校验失败不能进入 completed。传输状态与 Content-Length 校验由 yt-dlp 负责并通过错误回传，Rust 不重发媒体请求。
4. Rust 在重命名前持久记录目标路径、暂存路径和最终字节数，以**不覆盖**语义移动到最终文件名。建议 `<platform>_<video_id>_<task_id短标识>.<容器扩展名>`；冲突明确失败，不能先检查后用会覆盖的 rename。
5. 最终文件提交成功、SQLite completed 事务提交后才对外报告完成。数据库失败时保持可恢复的中断状态，不声称下载完成。

FFmpeg 全量解码是验收手段，不作为产品运行依赖。首版重试始终重新提取并写新暂存文件，不拼接遗留 `.part`，不接受前端自定义输出模板。

### 7.3 数据与恢复

| 表 | 字段 |
| --- | --- |
| `download_tasks` | id、platform（解析前可空）、source_url、video_id、title、author、status、revision、downloaded_bytes、total_bytes、staging_path、target_path、final_size_bytes、error_code、脱敏 error_message、created_at、updated_at、completed_at |
| `platform_settings` | platform、user_agent、非敏感代理设置、credential_ref、updated_at |
| `app_settings` | 下载目录、max_concurrent_downloads（默认 2，允许 1–3） |

SQLite 位于应用数据目录，由 Rust 独占写入；Python 不访问。进度在内存维护并节流保存，状态转换持久化。source_url 为可重新提取的作品链接，任务表不保存完整分享文案、原始 info dict、Cookie、代理密码或签名媒体 URL。Cookie 单独保存在平台设置表。

Cookie 按 2026-10-13 用户决定直接存 SQLite 的平台设置表，普通设置接口仅返回配置状态；设置编辑器通过专用命令读取已保存值。代理密码经系统安全存储保护，数据库只存引用；安全存储失败明确报错，不降级明文。关闭主窗口隐藏到托盘并继续当前任务；托盘“退出映取”和系统退出请求停止所有网络任务并等待引擎回收；重启将遗留 parsing/ready/queued/downloading 标为 failed/INTERRUPTED，不自动恢复下载。

文件移动与数据库事务不原子：对已记录提交意图的任务，启动时核对最终文件存在且大小匹配、暂存文件已消失，证据完整才补记 completed。无法证明时保留中断，不误删最终视频。已完成文件被外部删除时保留历史记录并提示缺失。

## 8. 网络和凭据边界

- 自有输入边界只接受两平台单作品链接；短链由对应 yt-dlp 提取器展开。禁用通用站点、用户主页、合集、直播、搜索和 playlist 路径，不把支持整个 yt-dlp 站点库作为产品能力。
- 代理按平台设置；显式直连不继承环境代理。HTTP/SOCKS5 都覆盖解析与媒体下载，设置修改不改变进行中任务。
- Cookie 仅发送到其所属平台的 HTTPS 域，跨域重定向不能携带不适用的 Cookie/Authorization。日志和前端事件不包含代理密码、Cookie、媒体 URL 或自由文本异常。
- 原有媒体目标、重定向与私网访问约束仍是迁移验收要求。移除旧 Rust HTTP 层后，检查必须覆盖 yt-dlp 的实际网络调用，不能仅保留 Rust 侧一个不再被使用的 URL 校验器。
- 固定提取器列表不等于完整网络目标校验。新实现须对实际提取/媒体目标和重定向验证域边界；直连须防止校验与连接采用不同 DNS 结果，并拒绝私网/回环目标。若固定 yt-dlp 版本没有满足边界的接入点，M1 明确阻塞，先形成最小适配方案，不静默取消已有检查。
- 用户配置的本地代理可以在 localhost，但不意味着允许媒体地址为 localhost。远端 DNS 代理下无法证明代理实际连接 IP，此限制单独记录。
- 不开放任意 shell、yt-dlp 参数、外部下载器或运行时插件；不引入监听端口、FastAPI、数据库服务、身份池或浏览器辅助服务。

## 9. 目录和打包

沿用现有工程目录；迁移后的核心布局为：

```text
python-engine/
  bridge.py               # 协议 v2 与单操作生命周期
  engine.py               # yt-dlp 适配、格式选择、Cookie 与事件映射
  requirements.in         # yt-dlp 固定基线
  requirements.txt        # 生成锁文件
  requirements-dev.in
  requirements-dev.txt
  pyinstaller.spec
src-tauri/src/
  python_bridge.rs        # 引擎进程、协议、退出与取消
  main.rs                 # M1 探针；桌面阶段按现有结构接入命令
  lib.rs                  # 任务与文件提交；按实际复杂度分文件
scripts/
  run_m1.py                # 当前统一验证入口
```

不保留 `vendor/dtk`、旧签名补丁和第二套 Rust HTTP 媒体下载器。测试目录保留有效协议、文件和任务行为用例，替换旧引擎专属测试。新来源清单记录 yt-dlp 版本、包哈希、Unlicense 与实际依赖许可证；删除旧代码时核对剩余资产，仍来自旧上游的文件继续保留适用声明。

分发项目自己的 Python/yt-dlp bridge，不是直接把官方 CLI 当作 JSONL sidecar。使用 PyInstaller onedir，检查 yt-dlp 的动态提取器与 hook 收集，随包保留完整依赖目录。打包在各目标系统/架构分别完成；用户无需安装 Python、uv 或 yt-dlp。

当前以 Tauri `bundle.resources` 目录映射分发整个 `video-engine/` onedir，通过 Rust 启动该目录内的引擎，不使用 `externalBin` 命名机制，不能只复制 exe。[Tauri 官方 sidecar 文档](https://v2.tauri.app/develop/sidecar/)

Windows 验证 WebView2、无终端窗口、非管理员、中文/空格路径与父进程退出；macOS 分别验证两种架构、嵌套签名与公证。通过安装包更新固定引擎版本，不做运行时自更新。源码成功不代表打包成功。

## 10. 迁移步骤与验收

### M1：替换旧引擎并验证核心链路

1. 用 yt-dlp 替换 engine 适配，实现预览/下载与协议 v2；Rust bridge 同步升级，改为管理独占操作进程及文件提交。
2. 删除第 2.2 节旧依赖、旧 HTTP 下载实现和失效测试；重新生成依赖锁、来源清单与 PyInstaller 配置。保留有价值的输入、文件、协议、取消用例，迁移到新实现。
3. 先串行验证双平台长短链预览与真实保存、文件解码、设置隔离、取消回收、敏感数据脱敏及网络边界。不得以 mock 或旧引擎结果替代。
4. 完成 Windows 打包 bridge 的原生探针验收；用户批准将全新未安装 Python 环境验收移至 M3，保留待验证状态。

退出条件：上述新链路全部有实际证据，旧引擎没有生产调用方、运行/构建依赖和兼容回退。缺少作品、凭据或网络条件时标为 NOT_RUN/UNAVAILABLE；旧 M1 状态不自动转 PASS。

### M2：桌面闭环

实现 Tauri/React、配置与凭据、队列、SQLite、预览/取消/手动重试、历史和二次确认删除。默认并发 2、上限 3；取消一个任务不影响其他任务；取消后无继续写入；重启历史有效；未知总大小展示正确；删除记录与文件操作分离。

### M3：安装分发

分别验证 Windows 11 x64、macOS Intel、macOS Apple Silicon 安装包，含进程回收、凭据、双平台真实下载、重启及签名。未运行的平台不得以其他平台成功替代。

### 最小验证矩阵

| 范围 | 关键用例与证据 |
| --- | --- |
| 输入/提取 | 双平台长短链、分享文本、多链接、主页/playlist/图集/直播拒绝，缺字段、删除/私密/网络失败分类 |
| yt-dlp 适配 | 真实提取器注册、格式限制、Cookie 域隔离、代理快照；不加载外部配置、插件或额外输出 |
| Bridge v2 | 分片、多行、错版本/ID、超限、坏 JSON、非零退出、EOF、重复/缺失终端事件 |
| 下载与文件 | 已知/未知长度、错误页面、截断、磁盘满、目录逃逸、同名不覆盖；hook finished 不提前完成 |
| 生命周期 | 解析超时、阻塞网络取消、stdin EOF、父进程异常退出、并发任务互不干扰 |
| 恢复/持久化 | 移动前后崩溃、数据库失败、重启、文件外部删除，重试从零 |
| 网络边界 | 平台后缀伪装、跨域重定向、私网目标、直连 DNS、HTTP/SOCKS5 及远端解析限制 |
| 打包 | 三目标分别实测，无 Python、中文路径、签名与引擎动态模块齐全 |

当前命令见 [README](../README.md)，验收证据见 [M1 验证](m1-validation.md)。M1 代码替换、离线测试与 Windows 打包已执行；抖音直连和 TikTok 代理下载均通过，全新无 Python Windows 环境未验证。M2 第一张卡的单任务数据库恢复已验证，见 [M2 验证记录](m2-validation.md)。并发调度验收见 [M2 队列验证](m2-queue-validation.md)；完整安装包测试尚未执行，M3 已配置标签触发的三平台构建/Release 工作流，实际 CI 与安装验收仍待执行。

## 11. 后续升级

升级固定 yt-dlp 版本时核对两平台提取器、网络层、格式/进度 hook、依赖与许可证变化；刷新锁文件与来源清单，运行适配/bridge/文件离线回归及双平台在线冒烟，再做对应安装包验收。失败不自动切换回 dtk；用户可回退到上一已验收的完整应用发布版本。

不沿用旧“签名模块替换”和 vendor 补丁回放流程。原工期包含 Rust HTTP 与旧上游裁剪，已不适用；按实际打包、网络边界和桌面工作量重新估算。

## 12. 权威来源

- [yt-dlp 官方项目](https://github.com/yt-dlp/yt-dlp)与[固定基线](https://github.com/yt-dlp/yt-dlp/tree/2026.08.19)：引擎、嵌入方式及选项语义。
- [固定版本依赖清单](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/pyproject.toml)：依赖与许可证。
- [本地真实下载记录](ytdlp-validation.md)：当前已验证能力；不替代后续安装验收。
- [DESIGN.md](../DESIGN.md)：产品视觉及已确认交互。
- [原始讨论](https://chatgpt.com/c/6acd9008-aba8-83e8-87bd-1e0d5e4630ba)与[旧 M1 记录](m1-legacy-validation.md)：历史来源；本次用户决定覆盖其旧引擎选择。

### 抖音详情请求补丁（2026-10-13 用户批准）

官方固定版本的 DouyinIE 仅发送 aweme_id，当前接口拒绝缺少 UIFID 和 x-secsdk-web-signature 的请求。自有 `python-engine/douyin.py` 仅补齐该详情端点参数，UIFID 来自授权 Cookie，不生成身份；沿用上游 parser 与 yt-dlp 下载器，不恢复旧依赖，不增加双引擎 fallback。实际响应使用的 `365yg.com` CDN 加入抖音域范围，继续执行每跳目标校验及直连 DNS 固定。来源、更新与移除条件见 [UPSTREAM](../vendor/UPSTREAM.md)，实测见 [M1 验证](m1-validation.md)。

### M2 第一张任务卡落地

用户批准先做单任务桌面闭环：Tauri 2 + React/TypeScript + shadcn/ui 亮色界面、分平台 Cookie/UA/代理、SQLite 任务记录、真实预览/下载/取消、重启记录恢复。默认并发 2、上限 3 的首版契约不变，在下一卡实现；本卡不显示并发、队列、重试、移除或文件删除按钮。

当前数据库实现采用 `download_tasks(id,snapshot)` 保存白名单展示快照，`commit_intents` 保存文件提交意图，`task_staging` 保存本应用独占暂存路径，`platform_settings` 保存平台 UA、Cookie、去凭据代理与系统凭据引用，`app_settings` 保存目录。Cover URL 仅在内存展示，不持久化短期 token。任务创建时复制两平台配置；下载使用该快照。与上文首版拟定列模型职责一致，并发阶段按实际查询需要扩展，不维护两套任务状态。

M2 开发可执行文件从仓库 `python-engine/dist/video-engine/` 启动既有打包引擎，无生产 Python 源码 fallback；Release 已通过 `tauri.release.conf.json` 将整个引擎目录映射到应用资源目录，GitHub Actions 按正式版本标签同步版本并构建三平台安装包；全新无 Python Windows 与两种 macOS 验收仍待完成。详情见 `m2-validation.md`。

### M2 第二张任务卡落地

Rust 使用每任务配置快照与独立取消/完成信号，下载槽位在引擎退出和清理结束后释放；FIFO 按提交顺序号选取，每次锁内预留槽位，解析独立且最多一个。`set_max_downloads(value)` 命令只接受 1–3，`get_settings` 返回 `max_downloads`；旧 app_settings 自动增加该列，默认 2，目录保持。任务快照增加 queued 状态与 queue_order，前端按 Rust 序号显示等待位置。降低上限保留已运行进程；退出先关闭调度，再取消并等待所有任务。详见 [本卡验收](m2-queue-validation.md)。

### M2 第三张任务卡落地

失败/取消且有作品身份的任务支持 `retry_task`：从 Rust 保存的 canonical_url 重新解析，新建任务并自动进入既有 FIFO 队列，使用最新配置快照，不沿用部分文件。无作品信息的解析失败/取消记录需重新粘贴链接。旧记录保留。

`remove_history` 仅删除终态记录和提交证据，不删除视频；`task_removed` 事件通知前端，已移除 ID 屏蔽迟到更新及旧快照。`delete_local_file` 由明确二次确认触发，只删除记录中的本地普通文件，保留记录与路径；不存在视为已删除，失败返回错误，列表及重启依据路径检测文件缺失。仍有进程或下载清理占用时拒绝历史操作。验收见 [M2 历史验证](m2-history-validation.md)。

### 解析预览与下载列表分离

任务快照增加 download_requested（旧快照缺省 false）；用户点击下载或手动重试时置 true。下载任务列表只展示已提交记录；纯解析/预览及其失败或取消仅在首页呈现。旧记录根据已有排队序号、下载字节、文件路径及 queued/downloading/completed 状态恢复已提交标记，不删除历史或文件。早期无上述证据的零字节失败记录无法可靠判断是否提交，不作为下载记录展示。

### 本地封面

引擎从重新提取的 thumbnails 按最大像素数（缺失尺寸时优先原始封面，再按上游偏好/排序）选封面，经同一 GuardedRH/代理/凭据/超时通道下载，最大 10 MiB，接受 JPEG/PNG/WebP。暂存 cover.part 后改名 cover，download_finished 可选返回 cover_filename/cover_bytes/cover_error。Rust 核验路径、大小和格式标识，以不覆盖方式保存 `<视频stem>.cover.<图片扩展名>`，SQLite 快照记录 cover_path/cover_error。封面失败不推翻视频成功，UI 提示封面未保存。通过任务 ID 读取本地封面，前端 Blob URL 在卸载时释放，重启后仍可展示。旧历史不自动补封面；删除视频保留封面供历史使用，移除记录不删文件。

抖音封面实测位于 p81-sign.douyinpic.com；仅将 douyinpic.com 加入抖音图片目标范围，仍校验每跳重定向、私网目标及平台隔离。

### 重复视频确认

以已完成历史中的 platform + video_id 为身份，解析预览成功时提醒。start_download 在同一队列锁下复核重复，未经 confirm_duplicate 返回 ALREADY_DOWNLOADED，状态保持 ready。重试自动解析后如存在已完成身份则停在 ready 等待确认，不自动入队。确认只针对当前任务 ID，重新下载使用原有唯一文件名，不覆盖旧文件。文件缺失但已完成记录仍在时继续提醒；移除历史后不另建隐含去重库。

本次验证：22 项 Rust 测试、Clippy、前端及桌面构建通过；原生 TikTok 验证自动提醒、暂不下载、后端拒绝未确认、确认后完成且旧文件不覆盖；Escape 与暂不下载均恢复下载按钮焦点。本机证据 `.local/duplicate-native-results.json`、`.local/duplicate-dialog.png`。

### 系统托盘与单实例

使用 Tauri 原生 TrayIcon/Menu，提供“显示窗口”和“退出映取”；左键图标恢复窗口。单实例采用首个注册的 `tauri-plugin-single-instance` 插件，第二次启动不进入业务初始化，通知已有实例显示、取消最小化并聚焦主窗口。退出复用 `Core::shutdown`，重入退出只发起一次清理；关闭窗口只隐藏，应用真正退出后不继续后台下载。macOS Reopen 恢复现有窗口。
