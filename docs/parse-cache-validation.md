# 可复用解析缓存验收

日期：2026-10-14。环境：macOS Apple Silicon；应用 `me.disign.yingqu`，引擎 `0.4.0`、协议 `3`，yt-dlp `2026.08.19`、DTK `5.1.3`（`4f0bed8483c35a980315d9c7b3a1d4a1119ad2b2`）。此前双引擎证据保留在 [dual-engine-validation.md](dual-engine-validation.md)。

## 行为契约

- 成功解析后，SQLite 事务保存白名单作品信息、视频封面和图集图片；图片使用 BLOB 保存原字节，不转码。成功提交后清理解析暂存目录，再发布 ready。
- `parse_cache` 按平台、作品 ID、实际 resolver 保存元数据及版本；`parse_cache_images` 保存图集字节；`parse_cache_aliases` 保存规范 URL 与身份映射。链接去除查询参数、片段和分享口令正文，不保存签名媒体 URL。
- 已知 URL 优先查 SQLite，命中不启动解析引擎。首次遇到另一条相同作品的链接时先识别身份：长链接纯本地识别，短链接只展开跳转；身份命中后不请求详情或媒体。
- 自动选择遵循视频 yt-dlp、图集 DTK；显式选择其他 resolver 不复用不同引擎的缓存。缓存版本包含 bridge、yt-dlp 和 DTK commit，版本不一致重新解析。
- 正常退出、重启后保留有完整缓存的 ready，首页恢复规范链接和预览。图集恢复到第一张，不保存临时图片选择位置。重启直接恢复预览；点击下载已下载作品时仍二次确认，重新解析后的重复提醒保留。
- 开始下载取得槽位后，同一 resolver 重新提取详情及有效媒体地址，核对作品身份、类型和图片张数；缓存不作为下载地址来源。重启后开始下载采用当前平台设置快照。
- 缓存独立于完成记录及下载目录；移除历史、删除下载文件不会隐式清除缓存。当前没有过期时间或清理设置，元数据可能反映上次解析时的内容。
- 初次升级从已有记录迁入元数据、可用封面及通过文件校验的完整图集。缺少图片的旧图集不能伪造完整预览，需要重新解析一次；旧元数据中未知的大小和清晰度继续显示未知。

## 离线验证

| 验证 | 结果 | 覆盖 |
| --- | --- | --- |
| `.venv/bin/python -m pytest -q` | PASS，121 项 | 双引擎、网络边界、bridge、图片下载；新增四组身份识别禁止调用详情、媒体和 DTK 提取 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --features desktop --lib -- --test-threads=1` | PASS，29 项 | URL 别名、图片/封面字节、resolver/版本隔离、迁移、缓存恢复、队列及文件提交 |
| 无引擎可执行文件的缓存预览 | PASS | 视频与图集在 shutdown/reopen 后仍 ready；封面及图片从 SQLite 读取；已知链接仍命中 |
| 无引擎时从缓存开始下载 | PASS | 正确失败 `ENGINE_UNAVAILABLE`，不把缓存命中伪报为下载成功 |
| 未知短链识别后复用缓存 | PASS | 首次只识别身份，保存别名；再次重启后无引擎也能命中 |
| 旧完成图集迁移、下载文件删除后读取预览 | PASS | 隔离测试数据库；SQLite 图像不依赖完成目录 |
| `npm run build` | PASS | TypeScript 与 Vite 构建 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | PASS | Rust 格式 |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings` | PASS | 静态检查 |

Rust 最终日志：`.local/parse-cache-rust-test.log`；clippy：`.local/parse-cache-clippy.log`。探针最初使用旧 0.3.0 二进制造成版本不匹配，重新构建后 Python 全套通过；未跳过或弱化测试。

## macOS 原生窗口与真实下载

使用此前用户授权的两平台 Cookie、UA；抖音直连、TikTok `socks5://127.0.0.1:3067`。不在文档或日志中记录 Cookie。测试窗口来自本次构建产物，没有覆盖 `/Applications/映取.app`。

| 作品 | 缓存与预览 | 重启恢复 | 从缓存预览下载 |
| --- | --- | --- | --- |
| 抖音 `7687900413543421811`，短链 `https://v.douyin.com/xhP440SkGa4/` | PASS，显示“解析缓存”，6 张图可切换；2160 × 3240 | PASS，正常退出后规范链接、ready、第一张图片恢复 | PASS，6 WebP，942668 字节 |
| TikTok `7690209438195633416`，短链 `https://vt.tiktok.com/ZSg1TvPEY/` | PASS，显示“解析缓存”，3 张图；1784 × 2396；再次相同短链立即返回缓存 | PASS，正常退出后规范链接、ready、图片恢复 | PASS，3 JPEG，753453 字节 |
| 抖音视频 `7696105473535135018` | PASS，规范视频链接直接缓存命中，标题、作者、10:28 时长及 SQLite 封面显示 | PASS，视频预览和封面恢复 | 本轮未重复下载此视频；缓存不代替下载的离线契约已验证 |

TikTok 下载任务窗口捕获“正在下载 0 / 3 张 240.1 KB / — 314.8 KB/s”，随后“1 / 3 张 241.1 KB”。抖音图集很快完成，本轮未捕获其中间字节进度，不能以完成状态代替进度证据。

最终安装包另外验证：重启直接显示视频缓存预览，不自动弹重复下载对话框；点击下载后仍出现原有二次确认。

本轮完成任务：

- 抖音：`e877cc01-b7fa-4c84-bfde-761add8ab539`。
- TikTok：`42a564ea-1ba8-447b-9e98-ad6ea9d32ec6`。

两任务保留 `cache_hit=true`；9 个文件逐一核对完成清单中的字节数及 SHA-256，全部匹配。原有 8 条成功记录保留，新增后共 10 条；再次重启完成页仍显示两条新记录和本地缩略图。测试样本提供高清资源，不能据此声称拿到了平台上传原图。

本机升级后的数据库有 7 个作品缓存，其中两组图集共 9 张图片、1696121 字节；迁移过程没有改写用户文件。数据库读取仅用于验收，不直接写入用户测试状态。

## 包与限制

- PASS：PyInstaller 构建、打包引擎握手 `0.4.0`、`.app` / `.dmg` 生成、`codesign --verify --deep --strict`、Info.plist 标识 `me.disign.yingqu`。仅使用 ad-hoc 签名，未使用 Developer ID 或公证。
- 构建：`APPLE_SIGNING_IDENTITY=- npm run tauri -- build --features desktop --config src-tauri/tauri.release.conf.json --bundles app,dmg -- --locked`。日志 `.local/parse-cache-macos-build.log`。
- 打包检查：`.venv/bin/python scripts/check_packaged_engine.py src-tauri/target/release/bundle/macos/映取.app/Contents/Resources/video-engine/video-engine`。
- 原生窗口基准 1440 × 900：缓存标记、图片预览、键盘操作、下载与重启 PASS。本轮 1280 × 800 未重跑；1024 × 680 沿用此前窗口控制不可用的 UNAVAILABLE 边界，不把基准窗口成功视为所有视口通过。
- Windows 与 macOS Intel：本轮 NOT_RUN。未验证 DMG 拖拽安装、跨架构运行或另一台机器首次启动。
