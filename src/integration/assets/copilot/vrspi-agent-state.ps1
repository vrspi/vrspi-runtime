# installed by vrspi
# managed by vrspi; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# VRSPI_INTEGRATION_ID=copilot
# VRSPI_INTEGRATION_VERSION=3

if ($(if ($env:VRSPI_ENV) { $env:VRSPI_ENV } else { $env:HERDR_ENV }) -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }))) { exit 0 }
if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_SOCKET_PATH) { $env:VRSPI_SOCKET_PATH } else { $env:HERDR_SOCKET_PATH }))) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { @{} } else { $inputText | ConvertFrom-Json }
} catch {
    $payload = @{}
}

function First-Text {
    param([object[]]$Names)
    foreach ($name in $Names) {
        $value = $payload.$name
        if ($value -is [string] -and -not [string]::IsNullOrWhiteSpace($value)) {
            return $value
        }
    }
    return $null
}

function Normalize-Event {
    param([string]$Event)
    if ([string]::IsNullOrWhiteSpace($Event)) { return "" }
    return $Event.Replace("_", "").Replace("-", "").ToLowerInvariant()
}

$eventName = First-Text @("hook_event_name", "hookEventName")
if ($eventName) {
    if ((Normalize-Event $eventName) -ne "sessionstart") { exit 0 }
} elseif (
    ($payload.PSObject.Properties.Name -contains "prompt") -or
    (First-Text @("tool_name", "toolName", "notification_type", "notificationType", "stop_reason", "stopReason", "reason"))
) {
    exit 0
}

$sessionId = $payload.session_id
if ([string]::IsNullOrWhiteSpace($sessionId)) {
    $sessionId = $payload.sessionId
}
if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$vrspi = if ([string]::IsNullOrWhiteSpace($(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }))) { "vrspi" } else { $(if ($env:VRSPI_BIN_PATH) { $env:VRSPI_BIN_PATH } else { $env:HERDR_BIN_PATH }) }
& $vrspi pane report-agent-session $(if ($env:VRSPI_PANE_ID) { $env:VRSPI_PANE_ID } else { $env:HERDR_PANE_ID }) --source vrspi:copilot --agent copilot --agent-session-id $sessionId --seq $seq 2>$null | Out-Null
