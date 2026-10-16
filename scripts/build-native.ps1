param([switch]$Release,[switch]$Offline)
$ErrorActionPreference='Stop'
$taskRoot=Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $taskRoot
$taskEncoded=[Environment]::GetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS','Process')
$taskRaw=[Environment]::GetEnvironmentVariable('RUSTFLAGS','Process')
try {
    $taskArguments=@('build','--locked','--target','x86_64-pc-windows-msvc')
    if($Offline){$taskArguments+='--offline'}
    if($Release){
        $taskArguments+='--release'
        if(([string]$taskEncoded+[string]$taskRaw).Contains('-crt-static')){throw '现有编译参数禁用了静态 CRT，请先明确解决该冲突。'}
        if($null -ne $taskEncoded){$env:CARGO_ENCODED_RUSTFLAGS=$taskEncoded+[char]31+'-C'+[char]31+'target-feature=+crt-static'}
        else{$env:RUSTFLAGS=($taskRaw+' -C target-feature=+crt-static').Trim()}
    }
    & cargo @taskArguments
    if($LASTEXITCODE -ne 0){throw '原生构建失败'}
}finally{
    [Environment]::SetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS',$taskEncoded,'Process')
    [Environment]::SetEnvironmentVariable('RUSTFLAGS',$taskRaw,'Process')
    Pop-Location
}
