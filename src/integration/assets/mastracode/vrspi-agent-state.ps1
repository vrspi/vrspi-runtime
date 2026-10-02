# installed by vrspi
# managed by vrspi; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# VRSPI_INTEGRATION_ID=mastracode
# VRSPI_INTEGRATION_VERSION=2

param([string]$Action = "")

if ($Action -notin @("session", "working", "idle", "blocked")) { exit 0 }
if ($(if ($env:VRSPI_ENV) { $env:VRSPI_ENV } else { $env:HERDR_ENV }) -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }))) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    $payload = $null
}

$sessionId = if ($null -ne $payload -and $payload.session_id -is [string]) { $payload.session_id } else { $null }
$seq = [DateTime]::UtcNow.Ticks
$vrspi = if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }))) { "vrspi" } else { $(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }) }
try {
    if ($Action -eq "session") {
        if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }
        & $vrspi pane report-agent-session $(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }) --source vrspi:mastracode --agent mastracode --seq $seq --session-start-source startup --agent-session-id $sessionId 2>$null | Out-Null
    } else {
        $args = @("pane", "report-agent", $(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }), "--source", "vrspi:mastracode", "--agent", "mastracode", "--state", $Action, "--seq", "$seq")
        if (-not [string]::IsNullOrWhiteSpace($sessionId)) {
            $args += @("--agent-session-id", $sessionId)
        }
        & $vrspi @args 2>$null | Out-Null
    }
} catch {
}
