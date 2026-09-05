# 在 Windows 上构建 AntifyBot（需已安装 Rust MSVC 工具链：https://rustup.rs）
# 用法：powershell -ExecutionPolicy Bypass -File build-win.ps1
$ErrorActionPreference = "Stop"
Set-Location "$PSScriptRoot"

Write-Host "-- 1/2 release 构建"
# --workspace：根目录的 cargo build 默认只构建根包，gui 成员必须显式带上
cargo build --release --workspace

Write-Host "-- 2/2 产物"
$cli = "target\release\antify-rs.exe"
$gui = "target\release\antify-gui.exe"
foreach ($f in @($cli, $gui)) {
    if (Test-Path $f) { Write-Host "  $((Get-Item $f).Length / 1MB -as [int]) MB  $f" }
}

Write-Host ""
Write-Host "桌面应用 : $gui（需 WebView2 运行时，Win11 自带）"
Write-Host "CLI 节点 : $cli  （serve / discover / send，--help 看用法）"
Write-Host "首次运行请在防火墙弹窗点「允许」。"
