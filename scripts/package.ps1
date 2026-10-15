# 打包成可以单独分发的单文件 exe。
#
#   pwsh -File scripts/package.ps1
#
# 产物：dist\code-count.exe  +  dist\使用说明.txt
#
# 打包完会做两项自检：
#   1. PE Subsystem 必须是 2（GUI）—— 否则双击还会弹控制台窗口
#   2. 不能依赖 VCRUNTIME140.dll / MSVCP140.dll —— 否则别人机器上可能启动失败

param(
    [string]$OutDir = "dist",
    [string]$ExeName = "code-count",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

if (-not $SkipBuild) {
    Write-Host "==> cargo build --release" -ForegroundColor Cyan
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "编译失败" }
}

$src = "target\release\$ExeName.exe"
if (-not (Test-Path $src)) { throw "找不到 $src" }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$dst = Join-Path $OutDir "$ExeName.exe"
Copy-Item $src $dst -Force

# ---------------------------------------------------------------- 自检 1：子系统
$bytes = [System.IO.File]::ReadAllBytes($dst)
$e_lfanew = [BitConverter]::ToInt32($bytes, 0x3C)
$subsystem = [BitConverter]::ToUInt16($bytes, $e_lfanew + 24 + 68)
$subsysName = switch ($subsystem) { 2 { "GUI" } 3 { "Console" } default { "未知" } }
if ($subsystem -eq 2) {
    Write-Host "OK  子系统 = 2 (GUI)，双击不会弹控制台" -ForegroundColor Green
} else {
    Write-Warning "子系统 = $subsystem ($subsysName)，双击仍会弹控制台窗口！"
    Write-Warning "检查 src\main.rs 顶部的 #![cfg_attr(not(debug_assertions), windows_subsystem = ""windows"")]"
}

# ---------------------------------------------------------------- 自检 2：CRT 依赖
$dumpbin = Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft Visual Studio" -Recurse -Filter dumpbin.exe `
    -ErrorAction SilentlyContinue | Where-Object { $_.FullName -like "*Hostx64\x64*" } | Select-Object -First 1

$crtPattern = "VCRUNTIME140|VCRUNTIME140_1|MSVCP140|api-ms-win-crt-"
if ($dumpbin) {
    $deps = & $dumpbin.FullName /dependents $dst 2>&1 | Where-Object { $_ -match "\.dll" }
    $bad = $deps | Where-Object { $_ -match $crtPattern }
    if ($bad) {
        Write-Warning "仍然动态依赖 CRT："
        $bad | ForEach-Object { Write-Warning "   $_" }
        Write-Warning "检查 .cargo\config.toml 里的 +crt-static，并且需要完整重新编译（cargo clean 后再 build）。"
    } else {
        Write-Host "OK  没有动态依赖 MSVC 运行库（静态链接生效）" -ForegroundColor Green
    }
    Write-Host "`n  实际依赖的系统 DLL：" -ForegroundColor DarkGray
    $deps | ForEach-Object { Write-Host "    $($_.Trim())" -ForegroundColor DarkGray }
} else {
    if ($bytes -match $crtPattern) {
        Write-Warning "在 exe 里找到了 VCRUNTIME/MSVCP 的字符串，可能仍动态依赖 CRT。"
        Write-Warning "装个 VS Build Tools 就能用 dumpbin 精确确认。"
    } else {
        Write-Host "OK  没找到 VCRUNTIME 依赖痕迹（dumpbin 不可用，用字符串扫描代替）" -ForegroundColor Green
    }
}

# ---------------------------------------------------------------- 使用说明
$readme = @"
代码行数统计
============

双击 code-count.exe 直接运行，不需要安装任何东西。

用法
----
1. 点右上角「选择文件夹」
2. 选中要统计的目录
3. 等几秒，结果就出来了

统计结果包含总行数、文件数、总大小，以及按扩展名分语言的排行。

会自动跳过这些目录（避免把 node_modules 之类的算进来）：
  .git / node_modules / target / dist / build / out / .next / .nuxt
  .venv / venv / __pycache__ / .cache / vendor / .gradle / .idea / bin / obj

二进制文件（前 8KB 里含 NUL 字节的）会被忽略，超过 64MB 的单个文件也会跳过。

系统要求
--------
Windows 10 1809 或更高版本，64 位。需要有显卡驱动支持 Direct3D 11。

运行库已经打包进 exe 里了，不用另外装 VC++ Redistributable。

打不开怎么办
------------
如果双击没反应或者闪一下就没了，多半是显卡相关的问题。
在 PowerShell 里跑一下，这样能看到错误信息：

  .\code-count.exe

（发布版是 GUI 子系统，本身不输出控制台信息；如果这里也没有任何输出，
说明是启动阶段就失败了，可以检查一下显卡驱动。）
"@
Set-Content -Path (Join-Path $OutDir "使用说明.txt") -Value $readme -Encoding UTF8

# ---------------------------------------------------------------- 汇总
$size = (Get-Item $dst).Length
Write-Host ""
Write-Host "==> 打包完成" -ForegroundColor Cyan
Write-Host "    $dst  ($([math]::Round($size/1MB, 2)) MB)"
Write-Host "    $(Join-Path $OutDir '使用说明.txt')"
Write-Host ""
Write-Host "把 $OutDir 文件夹整个发给别人，或者只发那个 exe 也行。" -ForegroundColor DarkGray
