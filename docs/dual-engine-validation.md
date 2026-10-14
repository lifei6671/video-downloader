# 双引擎与图集验收

日期：2026-10-14。本机 macOS 27.0.1（26A434）、Apple Silicon arm64。实际应用标识 `me.disign.yingqu`；引擎 0.3.0 / 协议 v3，yt-dlp 2026.08.19，DTK 5.1.3 / `4f0bed8483c35a980315d9c7b3a1d4a1119ad2b2`。

本次正式接入使用现有 GUI、任务队列、SQLite 和 GuardedRH/HttpFD。旧独立 prototype、手工 fixture 回放与旧单引擎 macOS/Windows 验收不是本次在线证据。

## 真实样本及配置

使用用户明确授权的本机 Cookie 文件，保存的两平台 Edge 150 UA；Cookie 原文不进入本文件、仓库、命令参数或日志。抖音直连，TikTok 独立 `socks5://127.0.0.1:3067`。

| 平台 | 分享链接 | 作品身份 | 内容 |
| --- | --- | --- | --- |
| 抖音 | `https://v.douyin.com/xhP440SkGa4/` | `7687900413543421811` | 知道你最怕这种了#寸头，@DG4114，6 张 |
| TikTok | `https://vt.tiktok.com/ZSg1TvPEY/` | `7690209438195633416` | #肌肉 #肌肉線條 #肌肉拉伤，@lychenzu1236，3 张 |

两条短链展开后自动选 DTK；元数据携带 kind=image_album / resolver=dtk，不因请求失败切换引擎。

## macOS 原生 GUI

| 项目 | 结果与实际观察 |
| --- | --- |
| 分享文案/短链解析 | PASS，抖音完整分享文案及两平台短链可解析 |
| 图片预览 | PASS，6/3 张本地预览、标题/作者/尺寸/来源/原声信息可见；点击第二张后 Tab + Return 选择第三张，图片与索引同步 |
| 来源真实性 | PASS，两个样本都没有明确原图候选，如实显示“高清”；未把高分辨率称为原图 |
| GUI 下载进度 | PASS，TikTok 捕获 0 B → 1.0 KB → 63.0 KB → 1/3 张、241.1 KB，显示实际速度；抖音捕获 0/6 张 → 2/6 张、261.2 KB。准确总量未知时显示“—”和不确定进度，不伪造百分比 |
| 正式多文件提交 | PASS，抖音 6 张 WebP 合计 942,668 bytes；TikTok 3 张 JPEG 合计 753,453 bytes，完整清单提交后才进入已完成 |
| 完成记录与封面 | PASS，显示图集类型、张数、总大小、独占目录及首张本地缩略图；两个旧视频记录和本地封面仍存在 |
| 本地图集查看器 | PASS，从已完成列表打开两平台图集，对话框可显示首张和末张，质量/尺寸与文件一致；Escape 关闭 |
| 正常重启恢复 | PASS，退出原包、新标识启动并迁移数据，再退出重启后记录、文件、首张缩略图及本地首末张查看保持；无自动续传 |
| 标识迁移 | PASS，Info.plist 为 me.disign.yingqu；原目录迁移至新应用数据目录，两平台 Cookie 已配置状态、UA、TikTok 代理和历史保持；迁移后实际再次解析/下载两条图集成功 |
| 重复下载 | PASS，提示二次确认；重新下载保存新的任务目录，旧目录及文件保留 |
| 暂存清理 | PASS，下载结束及退出后 task_staging 为 0，完成目录无 .part |
| 三设计视口 | PASS：1440×900（截图 2880×1800）与 1280×800（2560×1600），预览/完成列表/查看器可见且可操作；UNAVAILABLE：1024×680 仅启动和初始 AX 可读，后续控制持续超时/无窗口/无坐标，未取得可审查交互与截图，不能算通过 |

首轮图集任务：

- 抖音 `433fe6a9-3795-404b-a6f1-97638243fe15`，目录 `~/Downloads/Yingqu/douyin_7687900413543421811_433fe6a9-3795-404b-a6f1-97638243fe15/`。
- TikTok `ae3fdb76-7c62-4e7d-aa8d-ddf3aa34e5a9`，目录 `~/Downloads/Yingqu/tiktok_7690209438195633416_ae3fdb76-7c62-4e7d-aa8d-ddf3aa34e5a9/`。

补抓中间进度在最终新标识包内再次真实下载：抖音 `1f8b3ddb-fa9b-49da-93e3-cc206aecec5c`，TikTok `c9fface5-b287-4cb5-96c3-310adeb385f0`。因此当前完成列表为四个图集任务及两个旧视频任务，不误报为四个不同作品。

原标识包首轮下载后，新标识的系统路径缓存令 CUA 报旧标识、noWindowsAvailable 或 timeoutReached。经正常退出、使用完整验证包路径重新绑定后继续实际 GUI 验证；没有以数据库查询代替本地查看器通过。尺寸验证包覆盖初始窗口 width/height，复用同一业务代码、实际数据库和生产 sidecar，不使用 mock 数据。1024 包还尝试过独立测试启动标识并重新 ad-hoc 签名，未能解除工具阻塞，未作为正式产物或通过证据。临时验证包与配置已清理，正式 .app 恢复默认 1440×900 和 me.disign.yingqu，DMG 始终为正式默认配置。

## 文件读取证据

首轮全部文件重新计算 SHA-256，与 Rust 已保存 manifest 一致；实际 sips 读取尺寸与记录一致。文件头验证 WebP/JPEG，无转格式。首轮共 9 张；补抓进度两次下载分别保存另一套完整文件，不覆盖旧文件。最终再次核对四个图集任务共 18 个文件的 bytes/SHA-256/总量及无 .part，全部一致，task_staging 为 0。

| 平台/文件 | bytes | 实际尺寸 | SHA-256 |
| --- | ---: | --- | --- |
| 抖音 001.webp | 229884 | 2160×3240 | d63e4f45518683c1d5f2bd514f8b92f32653aaedf93662f2ac823e61f2fcc564 |
| 抖音 002.webp | 36534 | 1080×1920 | 1894d320c98e63362f1c609380bf6d7f1b50079626a5850a26e4736ddd338ab8 |
| 抖音 003.webp | 125828 | 2160×3240 | a09882a285eb83a2d66c3d00415e2fccf7334cdc7c8b5383bd619c8162539f35 |
| 抖音 004.webp | 193874 | 2160×3240 | 81af74d930fc4e035c11b825400582bab71546de3ffff1f07a9aa3aff7f04907 |
| 抖音 005.webp | 127454 | 2160×3240 | 9dc359a7fe477dd2c0092d93b3a441685d35e67a3aa94e5a49aa7503e5322455 |
| 抖音 006.webp | 229094 | 2160×3240 | b2f92e5f13ae307f35e5230178da04c10537060f63c4a4968a4a33b23ace4246 |
| TikTok 001.jpeg | 245826 | 1784×2396 | 041f504f746a84dfccb1b7c8fc1a5b706c757f98f1f47364b38c120c920ed600 |
| TikTok 002.jpeg | 255767 | 1812×2396 | 570435d64ed819759d317e5e2fff68f80f2114370c6016f967ade02f94f1d730 |
| TikTok 003.jpeg | 251860 | 1820×2396 | bae701aba22d87e6c7dd5ffd9970382cef09337825f656590815d12ef53d3406 |

高清只能证明本次平台返回资源及其尺寸，不能证明作者上传原文件。未通过猜测 CDN 模板寻找原图。

## 离线与打包检查

| 检查 | 结果 |
| --- | --- |
| Python pytest 全套 | PASS，117 passed in 27.10s，包含真实本地 HTTP 下载、原格式/累计进度、原图优先、404/410 降级及 403 不降级、签名/凭据网络边界、协议和上游更新保护 |
| Rust 全套串行 | PASS，25 passed，0 ignored；覆盖视频/图集提交、预览/重启、队列/取消/重试、数据库失败及只删除任务文件 |
| 图集提交意图恢复扩展回归 | PASS，模拟目录已移动、完成快照未写入，恢复全部 files、bytes 和张数；这不是线上强制崩溃试验 |
| clippy desktop / fmt | PASS，--all-targets -D warnings 与格式检查 |
| 前端 | PASS，npm run build（类型检查和 Vite） |
| 依赖 | PASS，uv pip check，生产/开发 universal 锁文件 |
| DTK vendor | PASS，固定源码重建与 --check；新 SHA 同步、补丁冲突保留源/manifest、未知依赖、本地修改拒绝的回归通过 |
| PyInstaller / 隔离 PATH 随包引擎 | PASS，完整 onedir、握手 v3 与固定双引擎版本 |
| macOS .app / DMG | PASS，arm64 构建及 codesign --verify --deep --strict；仅 ad-hoc，按用户要求无 Developer ID/证书/公证门禁 |

早期并行 Rust 回归曾因 fixture 暂存层级改变失败，已修正；并行还出现一次 APP_ALREADY_RUNNING，未将该次运行算 PASS。当前本地与 CI/Release 完整 Rust 门禁统一使用串行测试，避免这些真实进程/文件锁 fixture 的并发干扰，未删除或跳过有效用例。远端 CI 尚未运行。初始升级脚本 fixture 是合成 git 仓库，真实固定 upstream 的同步另行执行；未来任意上游提交不被本次结果预先保证兼容。

可复现门禁：

```sh
.venv/bin/python scripts/update_dtk_vendor.py --check
cargo build --locked --manifest-path src-tauri/Cargo.toml --bin m1-probe
.venv/bin/python -m pytest -q
cargo test --locked --manifest-path src-tauri/Cargo.toml -- --test-threads=1
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings
npm run build
.venv/bin/python -m PyInstaller --noconfirm --distpath python-engine/dist --workpath python-engine/build python-engine/pyinstaller.spec
APPLE_SIGNING_IDENTITY=- npm run tauri -- build --features desktop --config src-tauri/tauri.release.conf.json --bundles app,dmg -- --locked
codesign --verify --deep --strict src-tauri/target/release/bundle/macos/映取.app
.venv/bin/python scripts/check_packaged_engine.py src-tauri/target/release/bundle/macos/映取.app/Contents/Resources/video-engine/video-engine
```

## 验收边界

- `UNAVAILABLE`：两真实图集均不提供明确原图候选，不能报告原图在线下载成功。
- `NOT_RUN`：本次双引擎 Windows、macOS Intel、最低 macOS 13.3、互联网分发后 Gatekeeper、DMG 拖入 Applications 安装；旧单引擎证据不能代替。
- `NOT_RUN`：真实图集下载途中强制崩溃、真实平台取消/删除/重试交互；已有进程/离线文件与恢复回归通过，不扩大为线上 GUI PASS。
- `NOT_RUN`：DTK 视频路径在线完整 GUI 下载；两个旧视频的 DTK 详情独立解析通过，新路径单文件下载有本地真实 HTTP 回归。默认 yt-dlp 视频此前 macOS GUI 证据保留于 [macOS 验证](macos-validation.md)。
- 原声仅展示可获取的标题/作者，未实现独立音频下载或图集音乐合成。

上游接入与升级命令见 [双引擎方案](dtk-dual-engine-plan.md)、[开发文档](development.md#解析模块同步与补丁升级)及 [来源清单](../vendor/UPSTREAM.md)。
