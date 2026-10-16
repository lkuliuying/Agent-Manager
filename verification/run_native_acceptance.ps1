param([string]$Executable = 'target\x86_64-pc-windows-msvc\debug\agmanage.exe', [switch]$RequireAllSources)
$ErrorActionPreference='Stop'
Set-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
$taskExecutable=(Resolve-Path -LiteralPath $Executable).Path
function Invoke-TaskNative {
    param([string[]]$NativeArguments)
    $taskStart=[Diagnostics.ProcessStartInfo]::new()
    $taskStart.FileName=$taskExecutable
    $taskStart.WorkingDirectory=(Get-Location).Path
    $taskStart.UseShellExecute=$false
    $taskStart.CreateNoWindow=$true
    $taskStart.RedirectStandardOutput=$true
    $taskStart.RedirectStandardError=$true
    foreach($taskArgument in $NativeArguments){$taskStart.ArgumentList.Add($taskArgument)}
    $taskProcess=[Diagnostics.Process]::Start($taskStart)
    try {
        $taskStdout=$taskProcess.StandardOutput.ReadToEndAsync()
        $taskStderr=$taskProcess.StandardError.ReadToEndAsync()
        if(-not $taskProcess.WaitForExit(60000)){$taskProcess.Kill();throw '本次验证子进程超时，已仅关闭该子进程'}
        $taskOutput=$taskStdout.GetAwaiter().GetResult()
        $taskError=$taskStderr.GetAwaiter().GetResult()
        if($taskOutput){Write-Output $taskOutput.Trim()}
        if($taskError){Write-Output $taskError.Trim()}
        if($taskProcess.ExitCode -ne 0){throw "原生验证失败：$($NativeArguments -join ' ')"}
    }finally{$taskProcess.Dispose()}
}
function Get-OriginalAgentProcesses {
    $taskRows=@(Get-CimInstance Win32_Process -Filter "Name = 'ChatGPT.exe' OR Name = 'codex.exe' OR Name = 'OpenCode.exe' OR Name = 'DeepSeek Harness.exe'" | Where-Object {
        ($_.Name -eq 'ChatGPT.exe' -and $_.ExecutablePath -like '*OpenAI.Codex*') -or
        ($_.Name -eq 'codex.exe' -and $_.CommandLine -match '\bapp-server\b') -or $_.Name -in @('OpenCode.exe','DeepSeek Harness.exe')
    })
    $taskDesktopIds=@($taskRows | Where-Object Name -eq 'ChatGPT.exe' | ForEach-Object ProcessId)
    @($taskRows | Where-Object { $_.Name -eq 'codex.exe' -or $_.ParentProcessId -notin $taskDesktopIds } | ForEach-Object {
        '{0}:{1}' -f $_.ProcessId,$_.CreationDate.ToUniversalTime().Ticks
    })
}
$taskBefore=Get-OriginalAgentProcesses
$taskOldScale=[Environment]::GetEnvironmentVariable('SLINT_SCALE_FACTOR','Process')
try {
    foreach($taskScale in @('1','1.25','1.5','2')) {
        $env:SLINT_SCALE_FACTOR=$taskScale
        Invoke-TaskNative -NativeArguments @('--native-gate','--gate-output',"artifacts/native-gate/scale-$taskScale")
    }
}finally{[Environment]::SetEnvironmentVariable('SLINT_SCALE_FACTOR',$taskOldScale,'Process')}
Invoke-TaskNative -NativeArguments @('--capture-demo')
Invoke-TaskNative -NativeArguments @('--verify-ui')
Invoke-TaskNative -NativeArguments @('--smoke-live')
$taskLive=Get-Content -LiteralPath artifacts\native-live\result.json -Encoding utf8 | ConvertFrom-Json
$taskAfter=Get-OriginalAgentProcesses
$taskSame=@(Compare-Object $taskBefore $taskAfter).Count -eq 0
$taskCodex=$taskLive.sources | Where-Object kind -eq 'codex'
$taskUnexpectedDisconnects=@($taskLive.shutdownMetrics.disconnect_reasons.PSObject.Properties | Where-Object { $_.Name -ne 'manual_reconnect' -and [long]$_.Value -gt 0 }).Count
# 目录数量不能替代正文同步；异常重连不能当成有效的重连验收。
$taskPass=$taskCodex.sessions -ge 3 -and $taskCodex.freshSnapshots -ge 3 -and $taskCodex.messages -gt 0 -and $taskLive.metrics.connections -eq 2 -and $taskLive.shutdownMetrics.disconnect_reasons.manual_reconnect -eq 1 -and $taskUnexpectedDisconnects -eq 0 -and $taskLive.metrics.snapshots -ge 6 -and $taskLive.metrics.patch_batches -gt 0 -and $taskLive.metrics.revision_gaps -eq 0 -and $taskLive.metrics.controls_sent -eq 0 -and $taskLive.shutdownMetrics.subscriptions_closed -eq $true -and $taskLive.codexNavigationAvailable -eq $true -and $taskSame
$taskAllSources=@($taskLive.sources | Where-Object { $_.sessions -ge 2 -and $_.freshSnapshots -ge 2 -and $_.messages -gt 0 }).Count -eq 3
if($RequireAllSources){$taskPass=$taskPass -and $taskAllSources}
$taskSummary=[ordered]@{passed=$taskPass;allSourcesVerified=$taskAllSources;requiredAllSources=[bool]$RequireAllSources;unexpectedDisconnectReasons=$taskUnexpectedDisconnects;binarySha256=(Get-FileHash -LiteralPath $taskExecutable -Algorithm SHA256).Hash;live=$taskLive;sameOriginalProcesses=$taskSame;originalProcessCount=$taskBefore.Count;scaleCases=@('1','1.25','1.5','2');scaleMethod='process-local SLINT_SCALE_FACTOR, OS settings unchanged';screenshots='synthetic demo only'}
$taskSummary|ConvertTo-Json -Depth 8|Set-Content -LiteralPath artifacts\native-live\acceptance.json -Encoding utf8
$taskSummary|ConvertTo-Json -Depth 8 -Compress
if(-not $taskPass){throw '原生集成验收未通过，见 artifacts/native-live/acceptance.json'}
