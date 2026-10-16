param([switch]$SkipBuild,[switch]$Offline)
$ErrorActionPreference='Stop'
$taskRoot=Split-Path -Parent $PSScriptRoot
if(-not $SkipBuild){& (Join-Path $PSScriptRoot 'build-native.ps1') -Release -Offline:$Offline}
$taskSource=Join-Path $taskRoot 'target\x86_64-pc-windows-msvc\release\agmanage.exe'
if(-not (Test-Path -LiteralPath $taskSource -PathType Leaf)){throw '没有 Release 可执行文件'}
$taskStamp=[TimeZoneInfo]::ConvertTimeBySystemTimeZoneId([datetime]::UtcNow,'China Standard Time').ToString('yyyyMMdd-HHmmss')
$taskOutput=Join-Path $taskRoot "artifacts\agmanage-windows-x64-$taskStamp"
if(Test-Path -LiteralPath $taskOutput){throw '目标目录已存在，请稍后重试以保留原有包'}
New-Item -ItemType Directory -Path $taskOutput | Out-Null
$taskBinary=Join-Path $taskOutput 'AgentManager.exe'
Copy-Item -LiteralPath $taskSource -Destination $taskBinary
Copy-Item -LiteralPath (Join-Path $taskRoot 'README.md') -Destination (Join-Path $taskOutput 'README.md')
$taskPackage=[ordered]@{binary=$taskBinary;sha256=(Get-FileHash -LiteralPath $taskBinary -Algorithm SHA256).Hash;bytes=(Get-Item -LiteralPath $taskBinary).Length;platform='Windows x64';renderer='Slint / Winit / FemtoVG-WGPU';readOnly=$true;createdAtShanghai=$taskStamp}
$taskPackage | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskOutput 'manifest.json') -Encoding utf8
$taskPackage | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskRoot 'artifacts\latest-package.json') -Encoding utf8
$taskPackage | ConvertTo-Json -Compress
