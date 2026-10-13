# RoboCopy 批量搬运 · Rust + Tauri

调用 Windows 自带的 `robocopy.exe`（`/MT` 多线程）做高速批量搬运的桌面工具：
图形化任务管理、实时进度、日志完整落盘与节流展示、三级复制后校验、
`/MIR` 镜像二次确认、`/L` 试运行等。

源既可以是**整个目录**，也可以是**单个文件**：

- 目录：整体复制到目标目录下，并在目标端新建与源同名的子目录
  （`D:\照片\2024` → `E:\备份\2024`）；
- 单个文件：直接复制进目标目录、文件名不变，并自动关闭 `/MIR`（单文件没有镜像语义）。

技术栈：Tauri v2（Rust 后端 + WebView 前端），前端为 Vite + React + TypeScript + Tailwind CSS。

## 文档

- 产品文档（面向使用者）：[`docs/README.md`](docs/README.md)
  - [用户指南](docs/user-guide.md)：界面、任务编辑、复制参数、校验、日志、退出码、快捷键
  - [常见问题与排障](docs/faq.md)
- 协作与分层约定（面向 AI/协作者）：[`AGENTS.md`](AGENTS.md)
- 构建/打包/测试命令：见本文下方各节

## 目录结构

```
.
├── Cargo.toml              # workspace：crates/rc-core + src-tauri
├── crates/
│   └── rc-core/            # 纯逻辑库（不依赖 tauri，可独立单测）
│       └── src/
│           ├── models.rs  #   CopyTask/CopyOptions/TaskStatus/TaskEvent 数据契约
│           ├── winfs.rs    #   盘符枚举/容量/长路径/路径关系/格式化
│           ├── scanner.rs  #   源扫描 + TargetProgressTracker（hot/dirty/budget/prune）
│           ├── robocopy.rs #   命令拼装/OEM 解码/退出码/统计表解析/子进程执行
│           ├── verify.rs   #   三级校验 + 1MB 分块哈希（线程池并发）
│           └── engine.rs   #   调度线程 + worker + 进度轮询 + 日志节流落盘
├── src-tauri/              # Tauri bin：command 桥接 + 事件泵（唯一 UI 推送出口）
├── ui/                     # 前端：Vite + React + TS + Tailwind
├── build.ps1 / run.ps1     # 打包 / 启动脚本（.cmd 为双击包装层）
└── logs/                   # 运行产物：每次执行的完整日志（已 gitignore）
```

## 环境要求

- Rust 1.75+（stable）
- Node.js 18+ / npm
- Windows（复制依赖系统 robocopy.exe）

### 工具链说明（重要）

- `src-tauri/Cargo.toml` 的 `crate-type` **只保留 `rlib`**，切勿加回
  `staticlib` / `cdylib`：那两个是移动端产物，在 `x86_64-pc-windows-gnu`
  （MinGW）下链接 cdylib 会因导出的符号序号超过 PE 的 16 位上限而失败：
  `ld.exe: error: export ordinal too large`。
- MinGW（GNU）与 MSVC 工具链均可构建桌面版；如后续需要出移动端，
  请在 MSVC / macOS 环境下单独配置 crate-type。

## 构建与运行

### 一键脚本（推荐）

```powershell
.\run.cmd                 # 启动脚本：开发模式（Vite 热更新 + Tauri 窗口）
.\run.cmd -Release        # 直接运行已打包的 release 版本
.\run.cmd -Install        # 强制重装依赖后启动

.\build.cmd                          # 完整打包（含单元测试；生成 exe + NSIS 安装包）
.\build.cmd -SkipTest -NoBundle       # 快速出包：跳过测试、只产 exe（内网推荐）
.\build.cmd -Registry http://nexus.local.com:8081/repository/npm-proxy
```

`.cmd` 只是包装层（自动选用 pwsh、自动补 UTF-8 BOM、绕过执行策略），
也可以直接调用 PowerShell 脚本：`.\build.ps1 -SkipTest -NoBundle`。

> 注意：npm 默认走内网 registry 时可能 502，脚本默认使用官方源，
> 可用 `-Registry` 覆盖。

### 手动命令（等价）

```powershell
npm install                       # 安装 @tauri-apps/cli（workspace 根）
npm --prefix ui install           # 安装前端依赖
npm run tauri dev                 # 开发模式（自动起 Vite + cargo run）
npm run tauri build               # 打包（exe + NSIS 安装包）
cargo build --release -p robocopy-gui   # 只编译单文件 exe
```

也可手动分离运行（调试更直观）：

```powershell
# 终端 1
npm --prefix ui run dev           # Vite dev server → http://localhost:5173
# 终端 2
cargo run -p robocopy-gui         # 直连 devUrl 调试 Rust 侧
```

### 产物位置

| 产物 | 路径 |
|---|---|
| 可执行文件 | `target/release/robocopy-gui.exe` |
| NSIS 安装包 | `target/release/bundle/nsis/*.exe`（需联网下载 NSIS，内网常失败） |

单文件 exe 已内嵌前端资源，运行时不依赖 Node；目标机需具备 WebView2 运行时
（Win10/11 一般预装）。

### 运行日志

任务执行时完整日志落盘到工作目录下的 `logs/`（命名：时间戳 + 任务名 + 任务 ID），
UI 仅做节流展示。以 `-Release` 方式启动时工作目录固定为项目根，故日志写入 `./logs/`。

## 测试

```powershell
cargo test -p rc-core             # 纯逻辑单测：命令拼装/退出码/统计表解析/
                                  # 路径关系/扫描/校验/引擎端到端（真实 robocopy）
cd ui; npx tsc --noEmit           # 前端类型检查
```
