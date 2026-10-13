#Requires -Version 5.1
<#
.SYNOPSIS
  RoboCopy 批量搬运 —— 启动脚本（开发模式 / 运行 release 产物）

.DESCRIPTION
  默认开发模式：启动 Vite dev server + Tauri 窗口（前端热更新）。
  首次运行会自动补齐 npm 依赖。

.EXAMPLE
  .\run.ps1                 # 开发模式
  .\run.ps1 -Release        # 直接运行已打包的 release 版本
  .\run.ps1 -Install        # 强制重装依赖后启动
  .\run.ps1 -Registry http://nexus.local.com:8081/repository/npm-proxy

.NOTES
  若提示「禁止运行脚本」，请改用：
    powershell -ExecutionPolicy Bypass -File .\run.ps1
#>
[CmdletBinding()]
param(
    # 运行已打包的 release 版本（不启动 dev server）
    [switch]$Release,
    # 强制重新安装 npm 依赖
    [switch]$Install,
    # npm 源；内网代理不可用时可换成官方源
    [string]$Registry = "https://registry.npmjs.org"
)

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
Set-Location $root

function Write-Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }
function Write-Ok($text) { Write-Host "  [OK] $text" -ForegroundColor Green }
function Write-Warn2($text) { Write-Host "  [!] $text" -ForegroundColor Yellow }

function Assert-Command($name, $hint) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
        throw "未找到 $name，请先安装：$hint"
    }
}

function Ensure-Icon {
    $icon = Join-Path $root "src-tauri\icons\icon.ico"
    if (Test-Path $icon) { return }
    Write-Warn2 "缺少 src-tauri/icons/icon.ico，打包会失败（请放置一个 32x32 的 .ico）"
}

function Ensure-NodeDeps {
    $hasRootDeps = Test-Path (Join-Path $root "node_modules")
    $hasUiDeps = Test-Path (Join-Path $root "ui\node_modules")
    $needInstall = $Install -or (-not $hasRootDeps) -or (-not $hasUiDeps)
    if (-not $needInstall) { return }

    Write-Step "安装 npm 依赖（registry: $Registry）"
    & npm install --no-fund --no-audit "--registry=$Registry"
    if ($LASTEXITCODE -ne 0) { throw "根目录依赖安装失败（可尝试更换 -Registry 或检查网络）" }
    & npm --prefix ui install --no-fund --no-audit "--registry=$Registry"
    if ($LASTEXITCODE -ne 0) { throw "前端依赖安装失败（可尝试更换 -Registry 或检查网络）" }
    Write-Ok "依赖安装完成"
}

Assert-Command "cargo" "https://rustup.rs"
Assert-Command "npm" "https://nodejs.org"
Ensure-Icon

if ($Release) {
    $exe = Join-Path $root "target\release\robocopy-gui.exe"
    if (-not (Test-Path $exe)) {
        throw "未找到 release 产物：$exe`n请先执行 .\scripts\build.ps1 打包。"
    }
    Write-Step "启动 release 版本"
    Write-Ok $exe
    # 工作目录固定为项目根：日志写入 logs/
    Start-Process -FilePath $exe -WorkingDirectory $root | Out-Null
    exit 0
}

Ensure-NodeDeps
Write-Step "启动开发模式（Vite dev server + Tauri 窗口，Ctrl+C 退出）"
& npm run tauri dev
exit $LASTEXITCODE
