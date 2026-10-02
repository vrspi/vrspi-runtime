# installed by vrspi
# managed by vrspi; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# VRSPI_INTEGRATION_ID=droid
# VRSPI_INTEGRATION_VERSION=3

param([string]$Action = "")

if ($Action -ne "session") { exit 0 }
if ($(if ($env:VRSPI_ENV) { $env:VRSPI_ENV } else { $env:HERDR_ENV }) -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }))) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    $payload = $null
}

if ($null -eq $payload -or [string]::IsNullOrWhiteSpace($payload.session_id)) { exit 0 }

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$vrspi = if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }))) { "vrspi" } else { $(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }) }
try {
    & $vrspi pane report-agent-session $(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }) --source vrspi:droid --agent droid --agent-session-id $payload.session_id --seq $seq 2>$null | Out-Null
} catch {
}
