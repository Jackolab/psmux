# test_wait_for_blocks.ps1 -- wait-for blocks until signalled, latches a signal with no
# waiter, and a server exit releases a waiter (tmux semantics).
#   pwsh -NoProfile -File tests/test_wait_for_blocks.ps1 -Exe target/release/psmux.exe
param([string]$Exe = "$PSScriptRoot\..\target\release\psmux.exe", [string]$Tag = 'wf')
$ErrorActionPreference = 'Continue'
# This shell runs inside the installed Tenax host's psmux session: drop every inherited
# psmux/tmux variable so nothing here can reach that server or read its config.
foreach ($v in 'PSMUX_CONFIG_FILE', 'PSMUX_TARGET_SESSION', 'PSMUX_TARGET_FULL', 'PSMUX_SESSION', 'TMUX', 'TMUX_PANE', 'PSMUX_CLAUDE_TEAMMATE_MODE') { Remove-Item "env:$v" -ErrorAction SilentlyContinue }
$home2 = Join-Path ([IO.Path]::GetTempPath()) "psmux-wf-$Tag-$PID"
New-Item -ItemType Directory -Force $home2 | Out-Null
$env:USERPROFILE = $home2; $env:HOME = $home2
$env:LOCALAPPDATA = "$home2\AppData\Local"; $env:APPDATA = "$home2\AppData\Roaming"
New-Item -ItemType Directory -Force $env:LOCALAPPDATA, $env:APPDATA | Out-Null
$failed = 0
$L = "wf-$Tag-$PID"
& $Exe -L $L new-session -d -s x | Out-Null
function Start-Waiter($ch) {
  $psi = [System.Diagnostics.ProcessStartInfo]::new($Exe)
  foreach ($a in '-L', $L, 'wait-for', $ch) { $psi.ArgumentList.Add($a) }
  $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true
  [System.Diagnostics.Process]::Start($psi)
}
# a: a plain wait blocks until signalled
$w = Start-Waiter 'chA'
$alive = -not $w.WaitForExit(1500)
& $Exe -L $L wait-for -S chA | Out-Null
$done = $w.WaitForExit(10000)
"$Tag a: blocked-before-signal=$alive returned-after-signal=$done rc=$(if ($done) { $w.ExitCode } else { 'running' })"
if (-not ($alive -and $done)) { $script:failed++ }
if (-not $done) { $w.Kill() }
# b: a signal with no waiter is latched for the next wait
& $Exe -L $L wait-for -S chB | Out-Null
$w2 = Start-Waiter 'chB'
$t = [Diagnostics.Stopwatch]::StartNew(); $done2 = $w2.WaitForExit(10000)
"$Tag b: latched-wait-returned=$done2 in $($t.ElapsedMilliseconds) ms"
if (-not $done2) { $script:failed++ }
if (-not $done2) { $w2.Kill() }
# d: a waiter killed before the signal does not swallow it: the next wait returns
$dead = Start-Waiter 'chD'
$deadAlive = -not $dead.WaitForExit(1000)
$dead.Kill(); $dead.WaitForExit(10000) | Out-Null
& $Exe -L $L wait-for -S chD | Out-Null
$w4 = Start-Waiter 'chD'
$done4 = $w4.WaitForExit(10000)
"$Tag d: waiter-was-blocked=$deadAlive later-wait-got-the-signal=$done4"
if (-not ($deadAlive -and $done4)) { $script:failed++ }
if (-not $done4) { $w4.Kill() }
# c: the server's exit releases a waiter
$w3 = Start-Waiter 'chC'
$alive3 = -not $w3.WaitForExit(1000)
& $Exe -L $L kill-server | Out-Null
$done3 = $w3.WaitForExit(10000)
"$Tag c: blocked=$alive3 released-by-kill-server=$done3"
if (-not ($alive3 -and $done3)) { $script:failed++ }
if (-not $done3) { $w3.Kill() }

if ($failed) { throw "$failed wait-for check(s) failed" }
"wait-for: all checks passed"
