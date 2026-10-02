# managed by vrspi; reinstalling the integration replaces this file.
# VRSPI_INTEGRATION_ID=cursor
# VRSPI_INTEGRATION_VERSION=1

param([string]$Action = "")

function Exit-Hook {
    Write-Output "{}"
    exit 0
}

if ($Action -ne "session") { Exit-Hook }
if ($(if ($env:VRSPI_ENV) { $env:VRSPI_ENV } else { $env:HERDR_ENV }) -ne "1") { Exit-Hook }
if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }))) { Exit-Hook }

$inputText = [Console]::In.ReadToEnd()
$jsonStart = $inputText.IndexOf("{")
if ($jsonStart -gt 0) {
    $inputText = $inputText.Substring($jsonStart)
}
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    Exit-Hook
}

if ($null -eq $payload) { Exit-Hook }
$event = if ($payload.hook_event_name -is [string]) { $payload.hook_event_name } else { $payload.hookEventName }
if (-not [string]::IsNullOrWhiteSpace($event) -and $event -ne "sessionStart") { Exit-Hook }

$sessionId = $null
foreach ($name in @("session_id", "sessionId", "conversation_id", "conversationId")) {
    $value = $payload.$name
    if ($value -is [string] -and -not [string]::IsNullOrWhiteSpace($value)) {
        $sessionId = $value
        break
    }
}
if ([string]::IsNullOrWhiteSpace($sessionId)) { Exit-Hook }

$seq = [DateTime]::UtcNow.Ticks
$vrspi = if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }))) { "vrspi" } else { $(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }) }
try {
    & $vrspi pane report-agent-session $(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }) --source vrspi:cursor --agent cursor --seq $seq --agent-session-id $sessionId 2>$null | Out-Null
} catch {
}

Exit-Hook
