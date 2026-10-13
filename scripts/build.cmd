@echo off
rem ============================================================
rem  RoboCopy GUI - build wrapper (ASCII-safe, double-clickable)
rem  lives in scripts\ next to build.ps1 (project root = parent dir)
rem  - prefers PowerShell 7 (pwsh), falls back to Windows PowerShell 5.1
rem  - auto-fixes UTF-8 BOM on build.ps1 so PS 5.1 reads Chinese text correctly
rem  - passes through arguments, e.g.:  scripts\build.cmd -SkipTest -NoBundle
rem ============================================================
setlocal
cd /d "%~dp0"

set "PSH=powershell"
where pwsh >nul 2>nul
if %errorlevel%==0 set "PSH=pwsh"

%PSH% -NoProfile -ExecutionPolicy Bypass -Command "$p = Join-Path $PWD 'build.ps1'; $b = [IO.File]::ReadAllBytes($p); if ($b.Length -lt 3 -or $b[0] -ne 0xEF) { [IO.File]::WriteAllText($p, [Text.Encoding]::UTF8.GetString($b), (New-Object Text.UTF8Encoding($true))); Write-Host '[fix] added UTF-8 BOM to build.ps1' }; & $p %*"
set "RC=%errorlevel%"
endlocal & exit /b %RC%
