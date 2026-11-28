# test_control_client_detached.ps1 -- a control client that goes away tells the
# other control clients (%client-detached) and runs the client-detached hook,
# and list-clients honours -F (tmux semantics).
#   pwsh -NoProfile -File tests/test_control_client_detached.ps1 -Exe target/release/psmux.exe
param([string]$Exe = "$PSScriptRoot\..\target\release\psmux.exe", [string]$Tag = 'cd')
$ErrorActionPreference = 'Continue'
foreach ($v in 'PSMUX_CONFIG_FILE', 'PSMUX_TARGET_SESSION', 'PSMUX_TARGET_FULL', 'PSMUX_SESSION', 'TMUX', 'TMUX_PANE') { Remove-Item "env:$v" -ErrorAction SilentlyContinue }
$home2 = Join-Path ([IO.Path]::GetTempPath()) "psmux-cd-$Tag-$PID"
New-Item -ItemType Directory -Force $home2, "$home2\AppData\Local", "$home2\AppData\Roaming" | Out-Null
$env:USERPROFILE = $home2; $env:HOME = $home2
$env:LOCALAPPDATA = "$home2\AppData\Local"; $env:APPDATA = "$home2\AppData\Roaming"
$failed = 0
$L = "cd-$Tag-$PID"
& $Exe -L $L new-session -d -s x | Out-Null
& $Exe -L $L set-hook -g client-detached 'set -g @gone 1' | Out-Null
function Start-Control {
  $i = [System.Diagnostics.ProcessStartInfo]::new($Exe)
  foreach ($a in '-L', $L, '-C', 'attach', '-t', 'x') { $i.ArgumentList.Add($a) }
  $i.UseShellExecute = $false; $i.RedirectStandardInput = $true; $i.RedirectStandardOutput = $true; $i.CreateNoWindow = $true
  [System.Diagnostics.Process]::Start($i)
}
function Until([scriptblock]$cond) {
  for ($n = 0; $n -lt 100; $n++) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 100 }
  return $false
}
$watch = Start-Control
$read = $watch.StandardOutput.ReadToEndAsync()
$other = Start-Control
function Clients { @(& $Exe -L $L list-clients 2>$null | Where-Object { $_.Trim() }) }
$two = Until { (Clients).Count -ge 2 }
$pids = @(& $Exe -L $L list-clients -F '#{client_pid}' 2>$null | Where-Object { $_.Trim() })
$formatOk = $two -and $pids.Count -ge 2 -and ($pids | Where-Object { $_ -match '^\d+$' }).Count -eq $pids.Count -and ($pids -contains "$($other.Id)")
"$Tag 1: list-clients -F printed pids=$formatOk ($($pids -join ','))"
if (-not $formatOk) { $failed++ }
$other.Kill(); $other.WaitForExit(10000) | Out-Null
$one = Until { (Clients).Count -eq 1 }
$hook = Until { (& $Exe -L $L show-options -gv '@gone' 2>$null) -eq '1' }
"$Tag 2: killed client left list-clients=$one hook-ran=$hook"
if (-not ($one -and $hook)) { $failed++ }
$watch.StandardInput.Close()
$done = $read.Wait(10000)
$gotLine = $done -and ($read.Result -match '%client-detached')
"$Tag 3: remaining control client got %client-detached=$gotLine"
if (-not $gotLine) { $failed++ }
if (-not $watch.HasExited) { $watch.Kill() }
& $Exe -L $L kill-server | Out-Null
if ($failed) { throw "$failed control-client check(s) failed" }
"control-client detach: all checks passed"
