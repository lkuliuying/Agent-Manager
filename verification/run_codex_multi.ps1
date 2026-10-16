param(
    [ValidateRange(8, 45)][int]$Seconds = 35,
    [ValidatePattern('^[a-z0-9-]+\.json$')][string]$ResultName = 'codex-multi-three-sessions.json',
    [switch]$WaitForContent,
    [ValidateRange(60, 1800)][int]$WaitSeconds = 300
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)

function Get-TaskOriginalProcesses {
    $taskRows = @(Get-CimInstance Win32_Process | Where-Object {
        ($_.Name -eq 'ChatGPT.exe' -and $_.ExecutablePath -like '*OpenAI.Codex*') -or
        ($_.Name -eq 'codex.exe' -and $_.CommandLine -match '\bapp-server\b')
    })
    $taskDesktopIds = @($taskRows | Where-Object Name -eq 'ChatGPT.exe' | ForEach-Object ProcessId)
    # 只把 Desktop 主进程和现有 app-server 纳入连续性判断；渲染进程可自然增减。
    @($taskRows | Where-Object {
        $_.Name -eq 'codex.exe' -or $_.ParentProcessId -notin $taskDesktopIds
    } | Select-Object ProcessId, ParentProcessId, Name,
        @{n='CreatedUtc';e={$_.CreationDate.ToUniversalTime().ToString('o')}})
}

$taskBefore = Get-TaskOriginalProcesses
$taskProbeExit = 1
try {
    $taskProbeArgs = @('verification/codex_multi_probe.mjs', $Seconds, "verification/$ResultName")
    if ($WaitForContent) { $taskProbeArgs += @('wait-for-content', $WaitSeconds) }
    & node @taskProbeArgs
    $taskProbeExit = $LASTEXITCODE
} finally {
    $taskAfter = Get-TaskOriginalProcesses
    $taskBeforeKeys = @($taskBefore | ForEach-Object {
        '{0}:{1}' -f $_.ProcessId, ([datetime]$_.CreatedUtc).ToUniversalTime().Ticks
    })
    $taskAfterKeys = @($taskAfter | ForEach-Object {
        '{0}:{1}' -f $_.ProcessId, ([datetime]$_.CreatedUtc).ToUniversalTime().Ticks
    })
    $taskResult = [ordered]@{
        before = $taskBefore
        after = $taskAfter
        sameProcesses = (@(Compare-Object $taskBeforeKeys $taskAfterKeys).Count -eq 0)
        comparisonMethod = 'process_id_and_utc_creation_ticks'
    }
    $taskProcessName = $ResultName -replace '\.json$', '-processes.json'
    $taskResult | ConvertTo-Json -Depth 6 |
        Set-Content -LiteralPath "verification/$taskProcessName" -Encoding utf8
    [ordered]@{sameOriginalProcesses=$taskResult.sameProcesses; count=$taskBefore.Count} |
        ConvertTo-Json -Compress
}
exit $taskProbeExit
