# Agent 管理器

<!-- impeccable:product-schema 1 -->

## Platform

native-windows

用户当前在 Windows 使用多个 Agent，明确要求原生界面，不使用 WebView。其他平台尚未确定。

## Stack

已按预览审阅后的授权实现 Rust + Slint 1.18.1，使用 Winit 窗口和 FemtoVG-WGPU 原生绘制。正式工程位于根目录的 `Cargo.toml`、`src/`、`ui/` 和 `assets/`；运行与构建入口见 `README.md`。

## Users

同时使用多个 Agent 会话的本机用户。目标是减少切换窗口，集中查看消息和任务进度，保留现有 Agent 的使用方式。

## Capabilities and Constraints

- 接入 Codex Desktop，以及通过原进程观察插件接入的 OpenCode Desktop 1.18.27、DeepSeek Harness Desktop 0.2.0-rc.2，发现已有会话。
- 首期支持实时消息、任务状态、会话隔离和监控端断线重连。
- 只读监控，不向原会话发消息，不启动、恢复、接管或中断任务。
- 权限审批、任务控制、CLI、WSL 和远程接入未纳入本轮。
- 跨 Agent 混合总览，按来源独立连接、重连和标记同步状态。插件在接入管理中安装、检测、更新和卸载。
- Harness 历史读取使用只读查询，不调用可能激活未加载会话的 follow；不另起 headless runtime 充当桌面观察入口。
- 已通过早期三会话验证并完成预览审阅；实施顺序为关键原生能力验证、页面、真实会话接入。
- 总览优先运行中会话，可手动加入其他会话；切换页面和会话保留独立阅读状态。
- 预览使用明确标注的演示内容，不展示真实会话正文或标题。

## Evidence on Hand

`verification/codex-multi-continuing.json` 和配套进程记录证明当前安装版已有三个会话的发现、内容更新、增量版本隔离和验证器重连可用。协议为 Desktop 内部 IPC，仅验证了本机当前版本；不能据此承诺未来版本兼容、界面性能或完整历史读取能力。

正式原生验收入口为 `verification/run_native_acceptance.ps1`：真实原生窗口、中文鼠标选择与键盘复制、四种进程内渲染缩放、页面交互、真实已有会话订阅、监控端重连和原进程连续性。2026-10-15 的最终本地包验收记录见 `verification/native-acceptance-20261015.json`：5 个会话、965 条消息及工具记录、11 项原生交互检查通过，原进程连续且没有任务控制请求。包路径与摘要见 `artifacts/latest-package.json`。报告均不保存真实会话正文或标题；缩放模拟不能代替跨显示器或全新系统验收。

消息同时兼容旧 `turns` 和当前 `turnHistory.history.entitiesByKey`。快照和增量按会话、所有者、连接代次和版本隔离；批次失败保留旧快照并重新同步。正文仅存于运行内存。

2026-10-16 新增来源及 Codex 修复验证完成：Rust 契约测试 15 项、Node 插件测试 5 项、桌面自带 Cordis 的合成回归 2 项、原生交互检查 13 项及四档进程内缩放通过；两个合成来源通过真实 Windows 命名管道完成同名会话隔离测试。真实插件探测发现 OpenCode 45 个会话、读取 10 个快照，以及 Harness 28 个会话、读取 3 个快照，读取错误为 0。真实报告见 `artifacts/observer/after-harness-service-reload.json`，最终摘要见 `artifacts/observer/final-integration.json`；合成测试与真实结果分别记录。

跨应用文件统一使用 `%USERPROFILE%\.agmanage`：原 AppData 路径会被重定向到 Codex 私有缓存，普通独立检查也可能继承该视图，因此使用文件句柄核对物理路径。Harness 的可选 `agents` 服务通过 `context.get('agents')` 读取，避免 Cordis 拒绝未声明 inject 的属性访问。两项修复均经真实桌面重新加载验证。

Codex 反复重连的原因是管理器原 32 MiB 帧上限拒收约 53.8 MiB 的合法快照。已核对当前 Desktop 安装包 `26.1013.3656.0`（应用清单 `26.1013.31256`）的帧读取代码，将接收上限对齐为 256 MiB，并保留空帧、超限帧和无效 JSON 检查。回归覆盖大快照的分段接收及后续帧保留；诊断只记录固定错误分类与帧长度。协议证据见 `artifacts/native-live/codex-frame-contract.json`。

最终完整三来源验收见 `artifacts/native-live/acceptance.json`：原生窗口发现 Codex 3 个、OpenCode 45 个、Harness 28 个会话，共 2648 条已读取消息；分别取得 3、2、2 个实时正文快照。一次管理器主动重连前后共取得 6 个 Codex 快照、24 批增量，异常断线和版本缺口均为 0。退出已退订，15 个原应用进程保持连续，任务控制请求为 0。验收明确按来源检查正文快照，不能用目录条目代替正文读取，也不能把异常重连次数计为通过。

## Product Principles

- 同时看见多个会话的当前活动，并能集中阅读一个会话。
- 任务状态与监控连接状态分别呈现；断线时明确标注上次已知状态。
- 只有来源提供结构化步骤时才展示步骤进度，不推算完成百分比。
- 继续在各 Agent 的原应用操作任务，管理器承担观察职责。
