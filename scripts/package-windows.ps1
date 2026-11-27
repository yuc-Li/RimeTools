<#
.SYNOPSIS
    RimeTools 打包脚本（Windows）：便携 zip + 可选 NSIS 安装包。

.DESCRIPTION
    对应 Linux 侧的 scripts/package.sh。两种产物：

    1. 便携 zip（始终生成）
       解压即用，内含 exe、图标与一个安装/卸载说明。不写注册表，不装服务。

    2. NSIS 安装包（需要 tauri-cli）
       cargo install tauri-cli --version "^2"
       生成 <版本>_x64-setup.exe，按当前用户安装（不要求管理员）。

.PARAMETER NoBuild
    复用已有的 release 二进制，只打包。

.PARAMETER Installer
    额外生成 NSIS 安装包（会调用 cargo tauri build）。

.EXAMPLE
    .\scripts\package-windows.ps1

.EXAMPLE
    .\scripts\package-windows.ps1 -NoBuild -Installer
#>
[CmdletBinding()]
param(
    [switch]$NoBuild,
    [switch]$Installer
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

$version = (Get-Content package.json -Raw | ConvertFrom-Json).version
$arch = 'x64'
$outDir = Join-Path $root 'dist-pkg'
$exe = Join-Path $root 'src-tauri\target\release\rimetools.exe'

function Require-Command($name, $hint) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
        throw "找不到 $name。$hint"
    }
}

Require-Command cargo "请安装 Rust：https://rustup.rs/"

if (-not $NoBuild) {
    Write-Host '==> 构建前端'
    npm run build

    Write-Host '==> 构建 release 二进制'
    Push-Location src-tauri
    try {
        cargo build -p rimetools --release --features custom-protocol
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path $exe)) {
    throw "找不到 release 二进制：$exe（去掉 -NoBuild 重试）"
}
Write-Host "==> 使用二进制 $exe（$([math]::Round((Get-Item $exe).Length / 1MB, 1)) MB）"

if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
New-Item -ItemType Directory -Path $outDir | Out-Null

# ── 便携 zip ──
$portable = Join-Path $outDir "rimetools-$version-$arch"
New-Item -ItemType Directory -Path $portable | Out-Null
Copy-Item $exe (Join-Path $portable 'rimetools.exe')
Copy-Item 'src-tauri\icons\icon.ico' (Join-Path $portable 'rimetools.ico')

@'
RimeTools（便携版）
==================

双击 rimetools.exe 即可运行，无需安装。

依赖：
- Windows 10/11（64 位）
- WebView2 运行时：Windows 11 自带；Windows 10 若缺失，
  请安装 https://developer.microsoft.com/microsoft-edge/webview2/

数据位置：
- Rime 用户目录：%APPDATA%\Rime（小狼毫）
- 本工具设置：%LOCALAPPDATA%\rimetools 或应用数据目录
- 备份：默认与 Rime 用户目录分开存放，可在「设置」里查看

卸载：删除本文件夹即可（不会删除你的 Rime 词库）。
'@ | Set-Content -Path (Join-Path $portable 'README.txt') -Encoding UTF8

$zip = Join-Path $outDir "rimetools-$version-$arch.zip"
Compress-Archive -Path (Join-Path $portable '*') -DestinationPath $zip -Force
Write-Host "==> 便携包：$zip（$([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB）"

# ── NSIS 安装包 ──
if ($Installer) {
    if (-not (Get-Command cargo-tauri -ErrorAction SilentlyContinue)) {
        Write-Warning '未检测到 tauri-cli，跳过 NSIS。安装：cargo install tauri-cli --version "^2"'
    }
    else {
        Write-Host '==> 生成 NSIS 安装包（cargo tauri build）'
        Push-Location src-tauri
        try {
            cargo tauri build --bundles nsis
        }
        finally {
            Pop-Location
        }
        Get-ChildItem 'src-tauri\target\release\bundle\nsis' -ErrorAction SilentlyContinue |
            ForEach-Object { Write-Host "==> 安装包：$($_.FullName)" }
    }
}

Write-Host ''
Write-Host "完成，产物在 $outDir："
Get-ChildItem $outDir | Where-Object { -not $_.PSIsContainer } |
    ForEach-Object { Write-Host ("  {0}  ({1} MB)" -f $_.Name, [math]::Round($_.Length / 1MB, 1)) }