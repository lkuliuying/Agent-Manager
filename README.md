# Agent 管理器

Windows 原生的多 Agent 只读会话工作台，支持 Codex Desktop、OpenCode Desktop 和 DeepSeek Harness Desktop。界面使用 Rust + Slint 1.18.1 / Winit / FemtoVG-WGPU，保留浅色、青绿强调色与并排消息流。

2026-10-16 本机完整三来源验收通过：发现 Codex 3 个、OpenCode 45 个、Harness 28 个会话，三个来源均取得实时正文。Codex 大快照导致的反复重连已修复；验收期间异常重连为 0，管理器主动重连后继续同步，15 个原应用进程保持连续。来源筛选、混合总览及接入管理已实现，完整结果见 `artifacts/native-live/acceptance.json`。

## 使用

打开 Codex Desktop 中需要监控的会话，然后启动本地包内的 `AgentManager.exe`。管理器自动发现已有所有者的本机会话。

OpenCode 和 DeepSeek Harness 在“接入管理”中分别安装观察插件。自动检测失败时填写桌面程序的 exe 路径，再点击“检测”。插件随可执行文件内置，通过官方机制注册，无需安装全局 Node 或 CLI。OpenCode 需重新打开原应用加载；Harness 安装后先在原应用插件页重新启用观察插件，必要时重开应用。管理器不会自动重启原应用。Harness 的检测和安装还会校验实际依赖包文件与启用项，发现缺失或版本不一致时提示更新修复；文件校验通过不代表桌面已加载插件。

- 总览优先显示运行中的会话，可通过左侧眼睛／网格按钮将其他会话加入或移出总览；手动选择仅在本次运行保留。
- 搜索标题或项目名，按运行中、本轮完成、状态待确认筛选。
- 左侧可筛选来源；卡片与详情显示 Agent 名称。顶部连接状态及“接入管理”的重连、扫描操作按来源独立工作。
- 点击会话标题或“查看详情”集中阅读；窄窗口通过“任务信息”查看右侧信息。
- 正文支持原生鼠标选择及 `Ctrl+C`；复制按钮复制消息原文，长消息和工具输出可展开。
- 向上阅读时停止自动跟随；收到新消息后可点击“回到最新消息”。各会话和总览／详情分别保存阅读位置。
- 断线时保留最后内容、标注上次状态，自动重新连接；“立即重试”只重连管理器。
- “在 Codex 中查看”通过已注册的 `codex://threads/<id>` 导航。支持情况与当前安装版有关。
- OpenCode、Harness 详情提供“复制会话标识”，请在原应用继续操作。会话正文按关注或查看需要读取，目录发现不代表已读取完整历史。

管理器只订阅已有会话，不发送消息、执行命令、创建／恢复／中断任务或回复审批。关闭管理器会退订并关闭自己的连接，原任务继续在各自应用中运行。Harness 使用 `sessionQuery` 和公开事件，不使用本机版本中可能激活冷会话的 `sessionController.follow`。

正文仅缓存在本次运行的内存中。普通运行不会创建消息日志或离线历史文件。诊断页显示连接、同步与版本计数；验证报告仅保存计数，截图只使用演示数据。

观察通道使用当前用户专属、拒绝远程客户端的 Windows 命名管道。`%USERPROFILE%\.agmanage` 保存插件、安装路径、运行时通道地址，以及仅含加载阶段、固定错误码、版本和进程号的 `observer-health-*.json`。这个目录避免 AppData 的应用私有重定向，确保管理器与原应用看到同一份文件。旧版用户点击安装／更新即可迁移本项目注册项，旧目录保留。OpenCode 注册保留其他插件和 JSONC 注释；Harness 只修改 `desktop` profile。卸载只移除本项目注册项和内容仍与内置版本一致的受管文件；已加载的插件需原应用重启后完全退出。

## 构建与验证

需要 Windows x64、Rust MSVC 工具链和 C++ 构建工具。版本由 `Cargo.lock` 固定；缓存完整时可离线构建。

```powershell
.\scripts\build-native.ps1 -Offline
.\target\x86_64-pc-windows-msvc\debug\agmanage.exe

# 演示界面，不连接真实会话
.\target\x86_64-pc-windows-msvc\debug\agmanage.exe --demo

# 核心契约与原生交互检查
cargo test --offline --locked --target x86_64-pc-windows-msvc
node --test verification/observer.test.mjs
node verification/run_observer_fixture.mjs
.\target\x86_64-pc-windows-msvc\debug\agmanage.exe --verify-ui

# 包含剪贴板、四种渲染缩放、真实会话、重连和原进程连续性的本机验收
# 既有 Codex 验收要求至少三个已打开会话，其中有会话正在产生增量
.\verification\run_native_acceptance.ps1
# 在上述 Codex 条件下，两个新增来源各有至少两个已有会话时
.\verification\run_native_acceptance.ps1 -RequireAllSources

# 只读探测新增来源；报告只有计数，不保存会话正文或标识
.\target\x86_64-pc-windows-msvc\debug\agmanage.exe --probe-observers --seconds 15 --probe-output artifacts\observer\live-result.json

# 构建静态 CRT 的 Release 本地包
.\scripts\package-native.ps1 -Offline
```

系统剪贴板、Codex 索引和命名管道需要当前正常登录用户的桌面权限。打包应用的文件系统视图可能将 AppData 重定向，即使环境变量和包身份检查没有显示差异；跨应用文件验收使用文件句柄确认实际物理路径。受限令牌不能代表正常桌面插件的管道访问权限，真实接入及管道集成检查应在正常用户环境运行。缩放用进程内 `SLINT_SCALE_FACTOR` 验证，不修改系统设置；不能替代真实跨显示器拖动检查。

## 代码结构

- `ui/`：工作台、总览、详情、消息流、公共组件和原生能力验证窗口。
- `src/app.rs`：UI 线程上的选择、筛选、阅读状态和稳定模型更新。
- `src/state.rs`、`src/source.rs`：统一消息、任务状态、来源身份和会话键。
- `src/codex_state.rs`：Codex 两种历史结构的消息解析与原子增量处理。
- `src/monitor.rs`：只读索引、命名管道、来源校验、按会话重同步和退出清理。
- `src/observer.rs`、`src/installation.rs`：插件观察通道、版本校验、独立重连与安装管理。
- `plugins/`：OpenCode v1 插件、Harness Cordis 插件和共享本地观察实现。
- `src/native.rs`：系统协议发现和原应用导航。
- `verification/`：原有接入证据、去内容字段检查和原生验收脚本。

## 兼容范围

接入依赖 Codex Desktop 内部 IPC；当前本机安装包为 `26.1013.3656.0`，应用清单版本为 `26.1013.31256`，协议版本为 1。帧采用四字节小端长度前缀，接收上限与当前 Desktop 一致，为 256 MiB；空帧、超限帧和无效 JSON 仍会拒绝。索引或消息结构不兼容时会提示失败，不接管或恢复原任务。历史范围由来源提供，界面会显示是否完整。

新增来源的版本基线为 OpenCode Desktop `1.18.27` 和 DeepSeek Harness Desktop `0.2.0-rc.2`，插件观察协议为 1。不同版本会明确拒绝接入；CLI、WSL、远程服务不在首批范围。OpenCode 的项目级配置若覆盖全局插件列表，需要在原应用配置中保留本项目插件注册。

2026-10-16 的 Codex 反复重连已定位为管理器原 32 MiB 帧上限拒收约 53.8 MiB 的合法快照。修复后真实检查没有异常断线，主动重连及增量读取正常；报告中的 `disconnect_reasons` 只记录固定错误分类，`max_frame_bytes` 只记录长度。完整验收按来源检查已同步的正文快照，并拒绝把异常重连计为通过；Codex 仍须满足三个已打开会话的原有门槛。诊断及修复证据见 `artifacts/native-live/codex-reconnect-diagnostic.json`、`artifacts/native-live/codex-frame-fix-live.json`。

原生消息按标题、段落、列表、代码块显示，保留原文复制。当前代码块采用等宽文本；未实现 IDE 级语法着色或跨消息块的连续选择。

本地包未签名，未做全新 Windows 安装、其他 Desktop 版本或不同显卡的兼容验收；没有性能收益承诺。权限审批和任务控制暂未纳入。合成通道测试、原生交互测试与三个真实来源的验收分别记录，不能互相替代。
