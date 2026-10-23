# Nano CUDA 接入验证（2026-10-23）

范围：audio.cpp Windows CUDA、独立 FSMN-VAD、同一依赖卡 CPU/CUDA 选择、原有文案查看/复制/TXT 导出。协议 v3、引擎 0.12.0。未提交、推送、发布；没有修改用户原应用的模型选择或已有文案。证据目录 `.local/nano-cuda-delivery/`。

## 实际结果

| 状态 | 验证 | 结果与证据 |
| --- | --- | --- |
| PASS | Python 全量 | 415 passed；`python-final-reap.log`。包括 57 项模型/安装针对性回归及取消、超时、进程树、旧功能验证 |
| PASS | Rust 全量 | 113 passed、2 ignored；`rust-commit-final.log`。覆盖 backend/revision 持久化、旧结果兼容、互斥、取消、重启、独占文件句柄的短暂/持续占用 |
| PASS | Rust clippy | `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --features desktop --all-targets -- -D warnings`；`clippy-commit-final.log` |
| PASS | 前端构建 | `npm run build`，另隔离 Tauri Release 构建复用同一最终前端；`frontend-build.log`、`native-commit-final-build.log` |
| PASS | 原生 helper / sidecar | `native-build-final.log`、`pyinstaller.log`、`packaged-check-final.log`；内置 CPU 与独立 VAD，无模型权重随包；握手 0.12.0 |
| PASS | 固定下载与真实安装 | 使用前一轮完整下载的官方 ZIP/GGUF 缓存，经生产安装器校验完整大小/SHA-256、解压白名单、真实 CUDA 冒烟及提交；`real-cuda-install.log`。不冒充本轮完整网络下载 |
| PASS | 真实 HTTPS 来源 | 三个新资产经生产 HTTPS opener、受限重定向，返回 200、Content-Length 匹配，各读前 1024 bytes；`network-origin-check.json`。这仅验证下载入口，不替代整文件哈希 |
| PASS | CPU/CUDA 离线切换 | 原 CPU 模型保留，两个已安装版本重新校验权重并推理后切换，网络请求为零；`real-switch.json` |
| PASS | 独立 VAD | 原视频独立检测 79 段，与先前 CPU SRT 的 79 个区间全部一致；`source-vad.txt`、`vad-equivalence.json`。独立 helper 不加载 ASR 编码器或 LLM |
| PASS | 打包 CUDA 转录 | 3 秒无损视频真实输出“开放时间早上九点。”，打包 JSONL 与源码编排正文一致、原视频不变；`real-cuda-packaged.log` |
| PASS | 浏览器 UI | 35 组，1440×900、1280×800、1024×680；CUDA 选择/首次下载/不兼容/取消、旧模型身份、完整复制导出、长文案和加载滚动条。`browser-*.log`，三视口截图已查看 |
| PASS | 真实 Tauri | 专属标识 `me.disign.yingqu.nano-cuda-validation-20261023` 隔离实例；原视频转录、保存实际 CUDA 身份、重开、重启、取消保留旧文案、CPU/CUDA 切换。`native-final-test.log`、`native-cleanup-result.json`、`native-persistence.json`；取消后暂存登记为 0，源视频 SHA-256 不变 |

本机 i9-13900KF、RTX 4070 SUPER 12 GB。原始视频约 297.586 秒；源码生产编排一次约 **26.28 秒**，真实 Tauri 完整转录两次约 **22.25 / 28.26 秒**，包含音频提取与独立 VAD。后台测试负载不同，不把这些数字作为严格重复基准或通用速度承诺。先前当前 CPU 两轮均值 195.67 秒，仅作本机参考。

三处“降智”正确，之前 audio.cpp 默认分段漏掉的“然后你只能再次逃回短剧里去麻醉自己”已恢复。79 段均有输出不代表每段没有漏词、错字；未做全文独立人工听校，不声明全文准确率。

## 取消清理修复

真实 CUDA 取消测试发现：旧文案保持，但暂存清理第一次失败，TempDir 析构后目录消失、SQLite 登记残留。最终版本在返回调用方前显式终止 Windows Job、最多 5 秒等待全部子孙进程退出；已验证的自有目录仅对 Windows sharing/lock violation（32/33）作最多 310ms 重试，其他错误立即传播。清理失败不再被原取消结果掩盖。

独占句柄回归同时检查短暂占用后成功清理、持续占用明确失败并保留登记。真实 CUDA 最后一次取消约 **0.131 秒**，旧文案与 CUDA 选择保留，暂存登记清零；重启仍可读已保存文案。

## 环境问题与验收边界

- 沙箱内首次测试遇到临时目录权限/重命名问题；沙箱外隔离复核。旧下载探针在仓库 `.local` 临时目录仍出现 Windows error 32 共享占用，改用唯一系统临时目录后，原断言与全部 415 项均通过。没有删除断言、跳过失败测试或添加通用重试掩盖它。
- 普通 `npm run desktop:build` 的输出被用户正在运行的旧 `yingqu.exe` 占用，因此使用独立 Cargo 目标目录及专属测试 identifier 完成 Release 构建。未终止原应用；测试实例已关闭。不是安装包或升级安装验收。
- 真实 Tauri 的复制/TXT 按钮仅验证可点击命中，未写用户真实剪贴板或打开保存框；完整正文复制/导出由浏览器隔离测试验证，导出格式与文件保护由 Rust 测试验证。
- macOS Intel / Apple Silicon 本轮实际构建、运行：**NOT_RUN**。仅保留既有 CPU 路径，不以 Windows 成功替代。
- 其他 NVIDIA 显卡、全新无 Python Windows 安装、默认 identifier 安装包、完整新网络下载、全文人工听校：**NOT_RUN**。兼容 GPU 安装时仍必须真实 CUDA 冒烟。
- 既有脏工作区保留；新增实验、缓存、测试应用和证据均与用户原配置隔离。不会因接入 CUDA 自动重算已有文案或替用户切换到 CUDA。

## 安装提交阶段取消与退出

最终集成审查发现 Python 仅靠异常回滚无法覆盖进程在提交瞬间被终止。已由 Rust 持有安装前状态：失败/取消/退出时，在进程树回收后恢复旧目录和后端选择，再清理安装暂存；恢复失败则保留备份并报错。真实 sidecar fixture 分别停在旧目录移走后和选择写入后，新回归覆盖 CPU/CUDA、原目录有无、原选择有无、取消和退出，保持原测试断言。该验证是安装事务完整性测试，不替代新的完整模型网络下载。
