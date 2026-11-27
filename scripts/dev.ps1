<#
.SYNOPSIS
    RimeTools 开发脚本（Windows / PowerShell）。

.DESCRIPTION
    对应 Linux 侧的 scripts/dev.sh，用 PowerShell 实现同一套流程。

    - 默认开发模式：需要另一个终端跑 Vite（本脚本会提示）
    - -Prod：先构建前端，再以「资源已打包」的方式启动，启动更快、更接近发布形态

.EXAMPLE
    # 终端 1
    npm run dev
    # 终端 2
    .\scripts\dev.ps1

.EXAMPLE
    # 单窗口模式（不需要 Vite）
    .\scripts\dev.ps1 -Prod
#>
[CmdletBinding()]
param(
    [switch]$Prod
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

function Require-Command($name, $hint) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
        throw "找不到 $name。$hint"
    }
}

Require-Command node "请安装 Node.js 20+：https://nodejs.org/"
Require-Command cargo "请安装 Rust（含 MSVC 工具链）：https://rustup.rs/"

if (-not (Test-Path 'node_modules')) {
    Write-Host '==> 安装前端依赖（npm ci）'
    npm ci
}

if ($Prod) {
    Write-Host '==> 构建前端'
    npm run build
    Write-Host '==> 构建并启动（custom-protocol，前端已内嵌）'
    Push-Location src-tauri
    try {
        cargo run -p rimetools --features custom-protocol
    }
    finally {
        Pop-Location
    }
}
else {
    Write-Host '==> 启动开发构建（需要 Vite 已在 1420 端口运行）'
    Write-Host '    另一个终端请先执行：npm run dev'
    Push-Location src-tauri
    try {
        cargo run -p rimetools
    }
    finally {
        Pop-Location
    }
}