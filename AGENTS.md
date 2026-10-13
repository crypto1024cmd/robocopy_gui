# AGENTS.md —— 给 AI Agent 与协作者的协作指南

本文件描述本仓库（RoboCopy 批量搬运 · Rust + Tauri 版）需要遵守的约定：
目录结构、线程/事件模型、rc-core 与 UI 的边界、日志护栏与提交规范。
Agent 在采取任何可能影响这些约定的动作前，请先通读一遍。

> 历史说明：本仓库早期为 Python + tkinter 版本，现已整体替换为 Rust + Tauri。
> 若在代码或注释中看到「与原版一致 / Python 版」之类字样，指的是这次移植的对齐对象，
> 该 Python 源码已不在仓库内。

---

## 1. 项目目标与运行环境

- **目标**：Windows 下调用系统自带的 `robocopy.exe`（`/MT` 多线程），
  对「源目录 → 目标目录」做高速批量搬运，提供图形化任务管理、实时进度与三级校验。
- **运行环境**：Windows（依赖 `robocopy.exe` 与 WebView2）、Rust 1.75+、
  Node.js 18+。前端产物内嵌进单文件 exe，运行时不需要 Node。
- **启动方式**：`.\run.cmd`（开发模式）或 `.\run.cmd -Release`（运行已打包版本）。

## 2. 目录结构与代码归属

```
.
├── Cargo.toml                 # workspace：crates/rc-core + src-tauri
├── crates/rc-core/            # 纯逻辑库（不得依赖 tauri / UI）
│   └── src/
│       ├── lib.rs             #   模块导出 + 通用常量（LOG_* / LOG_DIR）
│       ├── models.rs          #   CopyTask / CopyOptions / TaskStatus / TaskEvent 数据契约
│       ├── winfs.rs           #   盘符枚举/容量/长路径/路径关系/格式化/robocopy.exe 定位
│       ├── scanner.rs         #   scan_source + TargetProgressTracker（hot/dirty/budget/prune）
│       ├── robocopy.rs        #   命令拼装/OEM 解码/退出码/统计表解析/子进程执行
│       ├── verify.rs          #   三级校验 + 分块哈希（线程池并发）
│       └── engine.rs          #   CopyEngine：调度、worker、轮询、日志节流落盘、事件投递
├── src-tauri/                 # Tauri bin：只做 command 桥接 + 事件泵
│   ├── tauri.conf.json        #   窗口尺寸/标题、devUrl、frontendDist、bundle 配置
│   └── src/
│       ├── main.rs / lib.rs   #   Builder、状态注册、退出清理钩子
│       ├── commands.rs        #   前端 invoke 的全部 command
│       └── events.rs          #   mpsc → emit("task-event") 的唯一推送出口
├── ui/                        # 前端：Vite + React + TS + Tailwind
│   └── src/
│       ├── types.ts           #   与 rc-core serde 契约对齐的 TS 类型
│       ├── api.ts             #   invoke / listen 封装（前后端唯一出入口）
│       ├── theme.ts           #   COLORS / 状态色 / 格式化函数
│       ├── App.tsx            #   装配：状态管理、事件订阅、日志缓冲、对话框调度
│       ├── components/        #   TopBar/TaskCard/RuntimePanel/LogConsole/… + ui/ 基础件
│       └── dialogs/           #   TaskEditor/CommandPreview/LogViewer/SettingsDialog
├── scripts/                   # 打包脚本：build.ps1 / build.cmd（.cmd 为双击包装层）
├── run.ps1 / run.cmd          # 启动脚本（.cmd 为双击包装层）
└── logs/                      # 运行产物：每次执行的完整日志（已 gitignore）
```

**铁律**：

- `crates/rc-core` 中任何文件 **不得依赖 `tauri`**，也不得引入 UI 框架，
  否则破坏可测试性并锁死非 GUI 场景。
- 与引擎交互只能通过 `TaskEvent` 事件；UI 不得绕过事件直读运行中的对象。
- `src-tauri` 只做桥接，不持有可变业务状态（状态全在 `CopyEngine`）。

## 3. 线程与事件模型

- 引擎 → UI 的唯一通道是 `std::sync::mpsc` 投递的 `TaskEvent`。
  线程模型见 `crates/rc-core/src/engine.rs` 顶部注释。
- **禁止** 让 worker / 轮询线程直接触碰 UI（Tauri 侧不存在这种能力，
  但同样禁止在 worker 里持有 `AppHandle`）。所有推送只能在事件泵线程里发生。
- **禁止** 让多个线程同时写同一个 `CopyTask` 实例的字段。状态变更只发生在
  该任务的 worker / 轮询线程中（每任务独立 `Mutex`）。
- 推给 UI 的事件里携带的是 `CopyTask` 的**快照副本**（`clone()`），UI 拿到的是副本。
- 运行状态（开始 / 结束）由 `TaskEvent::Running { running }` 显式表达。
  **禁止** 用「是否存在非终态任务」推断是否在跑 —— 待执行(PENDING)并不代表在跑，
  否则会在刚添加完任务时就把「开始」按钮禁用。
- 关闭程序时确保：`engine.shutdown()` 终止所有 worker → 等待调度线程退出 → 关闭窗口。

## 4. robocopy 命令拼装约束

相关实现集中在 `crates/rc-core/src/robocopy.rs`。改动前先看
`CopyOptions::normalized()` —— 它夹取了所有数值字段到合法范围。

- 线程数 `/MT:n` 必须夹到 `1..128`；`/R` 重试次数夹到 `0..20`；`/W` 等待秒数夹到 `0..120`。
- 源既可以是目录，也可以是**单个文件**。单文件走「父目录 + 文件名过滤」模式
  （`robocopy <父目录> <目标> <文件名> …`），**不得**附加 `/E` 或 `/MIR`；
  `split_file_source` 负责拆分，`source_kind` 负责判别。此时 `scan_source`
  的清单只含该文件、`root` 归一到父目录，镜像由引擎/UI 自动关闭。
  `path_relation` 据此放行文件源，但仍要阻止「源文件已位于目标目录内」（自我覆盖）。
- **有效目标根**：目录源要整体建到目标下（`<目标>\<源目录名>`）。因此命令拼装的 dest、
  进度轮询的 dest_root、复制后校验的 dest_root **必须统一**用
  `winfs::effective_dest_root(source, dest)` 计算，不得各自使用任务里的原始 dest，
  否则会出现「命令写进子目录、轮询却在父目录空转」的错位。单文件源与盘符根源
  回落到目标目录本身；该逻辑纯词法、不触碰文件系统，便于单测。
- 路径必须通过 `winfs::robocopy_path` 做 Windows 化
  （`/` → `\\`、长路径加 `\\?\` 前缀、UNC → `\\?\UNC\`）。
- `/MIR`（镜像）会删除目标端多余文件，必须有 UI 层二次确认
  （见 `ui/src/dialogs/TaskEditor.tsx`）。
- `/L`（试运行）必须保留「不写盘」语义，不要在试运行路径里出现任何真实写入；
  试运行也不启动进度轮询线程。
- **不要** 修改 robocopy 退出码语义：`0..7` 视为成功、`>=8` 判失败，
  高字节的 bit 含义与中文文案映射在 `robocopy.rs` 中已固定。
- 子进程用 `std::process::Command` + `creation_flags(0x08000000)`（CREATE_NO_WINDOW），
  stdout 按 8KB 增量解码（OEM 代码页，见 `encoding_rs`）。不要用阻塞式一次性读取。
- **不要** 修改 `src-tauri/Cargo.toml` 的 `crate-type`：只保留 `rlib`。
  加回 `staticlib` / `cdylib`（移动端产物）会在 MinGW 下因导出的符号序号超过 PE 的
  16 位上限而链接失败：`ld.exe: error: export ordinal too large`。
  MinGW（GNU）与 MSVC 工具链均可构建桌面版；如后续要出移动端，请在 MSVC / macOS
  环境下单独配置 crate-type。

## 5. 日志与可观测性

- 每次任务执行都会写一份完整日志到工作目录下的 `logs/`（命名见 `engine::new_log_path`），
  **完整内容始终落盘**，UI 仅做节流展示。
- UI 推送的日志最小间隔由 `LOG_EMIT_INTERVAL_MS`（40ms）控制，**不要** 在没有节流
  的情况下把 `robocopy` 的逐行输出刷上界面。
- 解析末尾统计表时使用 `LOG_TAIL_LINES`（300 行）的内存滑窗，不要把整份大日志常驻内存。
- 日志行单条上限 `MAX_LINE_CHARS = 400`，超过请截断并加 `…`，
  避免偶发超长路径把 UI 卡死。

## 6. 校验（Verify）语义

`verify::verify_task` 在复制完成后跑，按 `VerifyLevel` 取不同策略：

- `NONE`：跳过校验；UI 进度条直接 100%。
- `QUICK`：比对「大小 + 修改时间」。注意 FAT/exFAT 时间戳精度只有 2 秒，
  时间戳比较要走 `scanner::MTIME_TOLERANCE_NS` 的模糊匹配。
- `STRICT`：按 `HashAlgo`（MD5/SHA1/SHA256）算哈希并逐文件比对。
  **大文件哈希务必按块流式读取**（1MB 分块），不要一次性 `read()`。
- 校验并发走线程池（`verify::DEFAULT_WORKERS` 上限 8）；失败差异记录有上限，
  差异列表要可被查看 / 导出（见 `ui/src` 中任务详情面板与 `LogViewer`）。

## 7. UI 改动约定

- 主题色与状态色统一走 `ui/src/theme.ts` 的常量；新增控件**不要** 自己写死颜色，
  除非确有必要对齐某处既有硬编码。
- 所有与后端交互只能经 `ui/src/api.ts` 的 `invoke` / `listen` 封装；
  `types.ts` 的类型必须与 rc-core 的 serde 契约保持一致（`snake_case`）。
- 事件驱动：任务列表、总进度、按钮可用性都从 `task-event` 事件流派生，不要本地造状态。
- 任何会触发 I/O 或长时间计算的 command（如源大小估算、目录扫描）都应走
  `tauri::async_runtime::spawn_blocking` / `async`，不要阻塞主线程。
- 日志控制台按任务切换、自动滚动、可暂停；一次性大批量日志必须走缓冲批量刷新
  （见 `App.tsx` 的 60ms 批量刷新 + `LogConsole` 上限裁剪）。

## 8. 构建与本地验证

- `cargo test -p rc-core` 是核心自检（含真实 robocopy 端到端用例）：
  命令拼装 / 退出码 / 统计表解析 / 路径关系 / 扫描 / 校验 / 引擎端到端。
  任何对 `rc-core` 的修改至少跑一遍它。
- 前端改动用 `npx tsc --noEmit` 或 `npm --prefix ui run build` 验证。
- 端到端手工路径：建任务 → 跑小目录 → 检查 `logs/` 落盘、进度、校验差异、
  停止与退出清理。
- Windows 特有行为（UNC、跨盘符、长路径）若改了 `winfs.rs`，要跑一次小样本验证。
- 一键脚本：`.\scripts\build.ps1 -SkipTest -NoBundle`（快速出 exe）；
  打包脚本在 `scripts/` 下，用 `Split-Path -Parent $PSScriptRoot` 定位项目根，
  改动脚本时不要退回 `$PSScriptRoot` 当根目录。

## 9. 提交与风格

- 提交信息建议用 `动词(作用域): 描述`，如 `engine: 重试时清理残留 robocopy 子进程`。
- 引入新 crate 依赖前先确认是否真的需要 —— 默认「编辑现有文件」，
  保持依赖集最小、可审计；`rc-core` 的新依赖不得引入 UI / 运行时框架。
- 任何删改 `src-tauri/tauri.conf.json` 或 `rc-core::models` 数据契约的改动，
  请在 PR 描述里标明对 UI / 事件契约的影响。
- **不要** 提交 `logs/`、`target/`、`node_modules/`、`dist/`、`src-tauri/gen/`。
  `.gitignore` 已覆盖，但提交前自行复核 `git status`。

## 10. Agent 自查清单（提交前/响应前过一遍）

- [ ] 改动没有跨层越界（rc-core 没碰 tauri，UI 没绕过事件直读状态）
- [ ] 路径拼接都经过 `robocopy_path`，参数都过 `normalized()`
- [ ] worker / 轮询线程没有直接触碰 UI（推送只发生在事件泵线程）
- [ ] 运行状态判断只看 `TaskEvent::Running`，没有用非终态任务数量推断
- [ ] 日志节流、哈希分块读、长字符串截断 这些护栏没有被打穿
- [ ] `src-tauri/Cargo.toml` 的 `crate-type` 仍只有 `rlib`
- [ ] `.gitignore` 没有误提交 `logs/`、构建产物与缓存目录
- [ ] `cargo test -p rc-core` 通过，前端类型检查通过

按以上约定推进，能让这个工具保持「分层清晰、可单测、对 Windows 友好」的初衷。
