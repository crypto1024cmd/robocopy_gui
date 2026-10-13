#Requires -Version 5.1
<#
.SYNOPSIS
  RoboCopy 批量搬运 —— 打包脚本（单文件 exe + NSIS 安装包）

.DESCRIPTION
  依次执行：工具链检查 → 图标检查 → npm 依赖安装 → rc-core 单元测试
  → 前端类型检查与构建 → Tauri 打包（NSIS）。最后打印产物路径与大小。

.EXAMPLE
  .\build.ps1                        # 完整流程（含单元测试）
  .\build.ps1 -SkipTest              # 跳过单元测试，快速出包
  .\build.ps1 -SkipTest -NoBundle    # 只编译 exe，不打安装包（更快）
  .\build.ps1 -Registry http://nexus.local.com:8081/repository/npm-proxy

.NOTES
  若提示「禁止运行脚本」，请改用：
    powershell -ExecutionPolicy Bypass -File .\build.ps1
#>
[CmdletBinding()]
param(
    # 跳过 rc-core 单元测试
    [switch]$SkipTest,
    # 不生成 NSIS 安装包（只产出 exe，明显更快）
    [switch]$NoBundle,
    # 跳过 npm install（依赖已装好时）
    [switch]$SkipInstall,
    # npm 源；内网代理不可用时可换成官方源
    [string]$Registry = "https://registry.npmjs.org"
)

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
Set-Location $root
$started = Get-Date

function Write-Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }
function Write-Ok($text) { Write-Host "  [OK] $text" -ForegroundColor Green }
function Write-Warn2($text) { Write-Host "  [!] $text" -ForegroundColor Yellow }

function Assert-Command($name, $hint) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
        throw "未找到 $name，请先安装：$hint"
    }
}

function Invoke-Checked($label, [scriptblock]$action) {
    & $action
    if ($LASTEXITCODE -ne 0) { throw "$label 失败（退出码 $LASTEXITCODE）" }
}

# ---------------------------------------------------------------- 1. 工具链
Write-Step "检查工具链"
Assert-Command "cargo" "https://rustup.rs"
Assert-Command "npm" "https://nodejs.org"
& cargo --version
& node --version
Write-Ok "rustc / cargo / node / npm 就绪"

# ---------------------------------------------------------------- 2. 图标
Write-Step "检查应用图标"
$icon = Join-Path $root "src-tauri\icons\icon.ico"
if (-not (Test-Path $icon)) {
    throw "缺少应用图标：src-tauri\icons\icon.ico（请放置一个 32x32 的 .ico 后重试）"
}
Write-Ok "src-tauri/icons/icon.ico 就绪"

# ---------------------------------------------------------------- 3. 依赖
if ($SkipInstall) {
    Write-Step "跳过 npm 依赖安装（-SkipInstall）"
}
else {
    Write-Step "安装 npm 依赖（registry: $Registry）"
    Invoke-Checked "根目录依赖安装" { & npm install --no-fund --no-audit "--registry=$Registry" }
    Invoke-Checked "前端依赖安装" { & npm --prefix ui install --no-fund --no-audit "--registry=$Registry" }
    Write-Ok "依赖安装完成"
}

# ---------------------------------------------------------------- 4. 测试
if ($SkipTest) {
    Write-Step "跳过单元测试（-SkipTest）"
}
else {
    Write-Step "运行 rc-core 单元测试（含真实 robocopy 端到端）"
    Invoke-Checked "rc-core 测试" { & cargo test -p rc-core }
    Write-Ok "全部测试通过"
}

# ---------------------------------------------------------------- 5. 打包
if ($NoBundle) {
    Write-Step "编译 release exe（-NoBundle，跳过安装包）"
    Invoke-Checked "release 编译" { & cargo build --release -p robocopy-gui }
}
else {
    Write-Step "Tauri 打包（前端构建 + exe + NSIS 安装包）"
    & npm run tauri build
    if ($LASTEXITCODE -ne 0) {
        # NSIS 需要联网从 GitHub 下载，内网/代理环境经常超时；exe 本身已产出
        Write-Warn2 "Tauri 打包未全部完成（NSIS 安装包需联网下载，内网通常失败）。"
        Write-Warn2 "只需可执行文件时请改用： .\build.ps1 -NoBundle"
    }
    else {
        Write-Ok "安装包生成完成"
    }
}

# ---------------------------------------------------------------- 6. 产物
Write-Step "产物清单"
$exe = Join-Path $root "target\release\robocopy-gui.exe"
if (Test-Path $exe) {
    $size = [math]::Round((Get-Item $exe).Length / 1MB, 2)
    Write-Ok "可执行文件：$exe  ($size MB)"
}
$nsisDir = Join-Path $root "target\release\bundle\nsis"
if (Test-Path $nsisDir) {
    Get-ChildItem -Path $nsisDir -Filter *.exe | ForEach-Object {
        $size = [math]::Round($_.Length / 1MB, 2)
        Write-Ok "安装包：$($_.FullName)  ($size MB)"
    }
}

$elapsed = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)
Write-Host ""
Write-Host "打包完成，用时 $elapsed 秒。" -ForegroundColor Green
Write-Host "运行方式： .\run.ps1 -Release" -ForegroundColor Gray
