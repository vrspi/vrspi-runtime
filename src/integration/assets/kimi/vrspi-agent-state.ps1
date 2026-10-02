# installed by vrspi
# managed by vrspi; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# VRSPI_INTEGRATION_ID=kimi
# VRSPI_INTEGRATION_VERSION=7

param([string]$Action = "")

if (@("session", "working", "blocked", "idle") -notcontains $Action) { exit 0 }
if ($(if ($env:VRSPI_ENV) { $env:VRSPI_ENV } else { $env:HERDR_ENV }) -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }))) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    $payload = $null
}

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$sessionId = if ($null -ne $payload -and -not [string]::IsNullOrWhiteSpace($payload.session_id)) { $payload.session_id } else { $null }
$vrspi = if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }))) { "vrspi" } else { $(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }) }

try {
    if ($Action -eq "session") {
        if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }
        & $vrspi pane report-agent-session $(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }) --source vrspi:kimi --agent kimi --agent-session-id $sessionId --session-start-source startup --seq $seq 2>$null | Out-Null
    } else {
        if ([string]::IsNullOrWhiteSpace($sessionId)) {
            & $vrspi pane report-agent $(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }) --source vrspi:kimi --agent kimi --state $Action --seq $seq 2>$null | Out-Null
        } else {
            & $vrspi pane report-agent $(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }) --source vrspi:kimi --agent kimi --state $Action --agent-session-id $sessionId --seq $seq 2>$null | Out-Null
        }
    }
} catch {
}
