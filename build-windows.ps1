$ErrorActionPreference = 'Stop'

$uiRoot = $PSScriptRoot
$workspaceRoot = Split-Path -Parent $uiRoot
$projectRoot = Join-Path $workspaceRoot 'onee_checker'
if (-not (Test-Path -LiteralPath (Join-Path $projectRoot 'Cargo.toml') -PathType Leaf)) {
    throw "找不到 onee_checker 本體專案：$projectRoot（請將兩個專案放在同一上層目錄）"
}
$hostLine = (rustc -vV | Select-String '^host:').ToString()
if (-not $hostLine) {
    throw '無法判斷 Rust host target。'
}
$hostTarget = $hostLine.Split(':', 2)[1].Trim()

cargo build --release --manifest-path (Join-Path $projectRoot 'Cargo.toml') --bin oneechk
if ($LASTEXITCODE -ne 0) {
    throw 'oneechk CLI 建置失敗。'
}

cargo build --release --manifest-path (Join-Path $uiRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) {
    throw 'Onee Checker UI 建置失敗。'
}

$cliBinary = Join-Path $projectRoot "target\$hostTarget\release\oneechk.exe"
$uiBinary = Join-Path $uiRoot "target\$hostTarget\release\oneechk-ui.exe"
if (-not (Test-Path -LiteralPath $cliBinary -PathType Leaf)) {
    throw "找不到建置完成的 CLI：$cliBinary"
}
if (-not (Test-Path -LiteralPath $uiBinary -PathType Leaf)) {
    throw "找不到建置完成的 UI：$uiBinary"
}

$dist = Join-Path $uiRoot 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item -LiteralPath $cliBinary -Destination (Join-Path $dist 'oneechk.exe') -Force
Copy-Item -LiteralPath $uiBinary -Destination (Join-Path $dist 'oneechk-ui.exe') -Force
Copy-Item -LiteralPath (Join-Path $uiRoot 'assets\OFL.txt') -Destination (Join-Path $dist 'NOTICE-Noto-Sans-TC.txt') -Force
Write-Output "Windows app bundle ready: $dist"
