# test_control_mixed_versions.ps1 -- a control client and a server from different
# builds still talk: new client on an old server enters control mode, and a new
# server reports an old client's pid in list-clients -F '#{client_pid}'.
#   pwsh -NoProfile -File tests/test_control_mixed_versions.ps1 -Exe target/release/psmux.exe -OldExe <pinned psmux.exe>
param([string]$Exe = "$PSScriptRoot\..\target\release\psmux.exe", [Parameter(Mandatory)][string]$OldExe, [string]$Tag = 'mv')
$ErrorActionPreference = 'Continue'
foreach ($v in 'PSMUX_CONFIG_FILE', 'PSMUX_TARGET_SESSION', 'PSMUX_TARGET_FULL', 'PSMUX_SESSION', 'TMUX', 'TMUX_PANE') { Remove-Item "env:$v" -ErrorAction SilentlyContinue }
$home2 = Join-Path ([IO.Path]::GetTempPath()) "psmux-mv-$Tag-$PID"
New-Item -ItemType Directory -Force $home2, "$home2\AppData\Local", "$home2\AppData\Roaming" | Out-Null
$env:USERPROFILE = $home2; $env:HOME = $home2
$env:LOCALAPPDATA = "$home2\AppData\Local"; $env:APPDATA = "$home2\AppData\Roaming"
$failed = 0
function Start-Control([string]$client, [string]$sock) {
  $i = [System.Diagnostics.ProcessStartInfo]::new($client)
  foreach ($a in '-L', $sock, '-C', 'attach', '-t', 'x') { $i.ArgumentList.Add($a) }
  $i.UseShellExecute = $false; $i.RedirectStandardInput = $true; $i.RedirectStandardOutput = $true; $i.CreateNoWindow = $true
  [System.Diagnostics.Process]::Start($i)
}
function Until([scriptblock]$cond) {
  for ($n = 0; $n -lt 100; $n++) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 100 }
  return $false
}

$L = "mv-old-$Tag-$PID"
& $OldExe -L $L new-session -d -s x | Out-Null
$c = Start-Control $Exe $L
$read = $c.StandardOutput.ReadToEndAsync()
$c.StandardInput.WriteLine('display-message -p mixed-ok')
$c.StandardInput.Close()
$done = $read.Wait(10000)
$ok = $done -and ($read.Result -match '(?m)^%begin [^\r\n]*\r?\nmixed-ok\r?\n%end ')
"$Tag 1: new client on old server runs a control command=$ok"
if (-not $ok) { $failed++ }
if (-not $c.HasExited) { $c.Kill() }
& $OldExe -L $L kill-server | Out-Null

$L = "mv-new-$Tag-$PID"
& $Exe -L $L new-session -d -s x | Out-Null
$o = Start-Control $OldExe $L
$up = Until { @(& $Exe -L $L list-clients 2>$null | Where-Object { $_.Trim() }).Count -ge 1 }
$pids = @(& $Exe -L $L list-clients -F '#{client_pid}' 2>$null | Where-Object { $_.Trim() })
$pidOk = $up -and ($pids -contains "$($o.Id)")
"$Tag 2: new server reports the old client's pid=$pidOk ($($pids -join ','); client $($o.Id))"
if (-not $pidOk) { $failed++ }
$o.Kill()
& $Exe -L $L kill-server | Out-Null
if ($failed) { throw "$failed mixed-version check(s) failed" }
"control mixed versions: all checks passed"
