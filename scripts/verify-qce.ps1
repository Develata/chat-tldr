#Requires -Version 7.2
<#
.SYNOPSIS
Verify a completed QCE single-file JSON export using isolated, offline CLI commands.
.DESCRIPTION
All captures, the isolated profile, and receipt.json stay in one new private run
directory. Source exports are read only. No model analysis, SQL access, or cleanup
is performed. Raw captures can contain private chat text: do not publish them.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $InputFile,
    [string] $CliPath,
    [string] $EvalPath,
    [string] $OutputRoot,
    [string] $SelfUid,
    [string] $SelfUin,
    [ValidateRange(1, 3600)][int] $TimeoutSeconds = 120
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$suffix = if ($IsWindows) { '.exe' } else { '' }
if (-not $CliPath) { $CliPath = Join-Path $repo "target/debug/chat-tldr$suffix" }
if (-not $EvalPath) { $EvalPath = Join-Path $repo "target/debug/chat-tldr-eval$suffix" }
if (-not $OutputRoot) { $OutputRoot = Join-Path $repo 'private/acceptance' }
$utf8 = [System.Text.UTF8Encoding]::new($false, $true)
$script:checks = [System.Collections.Generic.List[object]]::new()
$script:commands = [System.Collections.Generic.List[object]]::new()
$script:errors = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
$script:runDirectory = $null
$script:profile = $null
$sourcePath = $null
$sourceBefore = $null
$sourceAfter = $null
$cliHash = $null
$evalHash = $null
$cliHashAfter = $null
$evalHashAfter = $null
$firstStats = $null
$repeatStats = $null
$chatCount = 0
$messageCount = 0L
$sourceUnchanged = $false
$success = $false
$started = [DateTimeOffset]::UtcNow

function Stop-Acceptance([string] $Code) {
    [void] $script:errors.Add($Code)
    throw [System.InvalidOperationException]::new($Code)
}

function Assert-Check([string] $Name, [bool] $Passed) {
    $script:checks.Add([ordered]@{ name = $Name; passed = $Passed })
    if (-not $Passed) { Stop-Acceptance "A_$($Name.ToUpperInvariant())" }
}

function Resolve-FilesystemPath([string] $Path) {
    # PowerShell's Set-Location does not update Environment.CurrentDirectory.
    # Resolve through its provider API so relative paths and PS drives follow PWD.
    $provider = $null
    $drive = $null
    try {
        $resolved = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath(
            $Path, [ref] $provider, [ref] $drive
        )
    } catch { Stop-Acceptance 'A_PATH_RESOLUTION' }
    if ($provider.Name -cne 'FileSystem') { Stop-Acceptance 'A_PATH_NOT_FILESYSTEM' }
    return $resolved
}

function Save-NewText([string] $Path, [string] $Text) {
    $file = [System.IO.File]::Open($Path, [System.IO.FileMode]::CreateNew)
    try {
        $bytes = $utf8.GetBytes($Text)
        $file.Write($bytes, 0, $bytes.Length)
    } finally { $file.Dispose() }
}

# Arguments never enter a shell. Copy both pipes concurrently as bytes, preserving
# UTF-8 without PowerShell's native-command encoding or stderr formatting changes.
function Invoke-Captured([string] $Id, [string] $Executable, [string[]] $Arguments, [string] $Tool) {
    $capture = [ordered]@{
        id = $Id; tool = $Tool; exit_code = $null; timed_out = $false
        stdout = "$Id.stdout.jsonl"; stderr = "$Id.stderr.log"; runner_error = $null
    }
    $script:commands.Add($capture)
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo.FileName = $Executable
    $process.StartInfo.WorkingDirectory = $script:runDirectory
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $process.StartInfo.ArgumentList.Add($argument) }
    # Fresh config uses these environment names. Do not pass real model keys even
    # though all analysis invocations below require --dry-run.
    foreach ($key in @('TYPESAFE_API_KEY', 'CHAT_TLDR_LLM_API_KEY', 'CHAT_TLDR_EMBED_API_KEY')) {
        [void] $process.StartInfo.Environment.Remove($key)
    }
    $stdout = $null
    $stderr = $null
    $startedProcess = $false
    $cancel = [System.Threading.CancellationTokenSource]::new()
    try {
        $stdout = [System.IO.File]::Open((Join-Path $script:runDirectory $capture.stdout), [System.IO.FileMode]::CreateNew)
        $stderr = [System.IO.File]::Open((Join-Path $script:runDirectory $capture.stderr), [System.IO.FileMode]::CreateNew)
        $startedProcess = $process.Start()
        if (-not $startedProcess) { throw 'Process did not start' }
        $outTask = $process.StandardOutput.BaseStream.CopyToAsync($stdout, 81920, $cancel.Token)
        $errTask = $process.StandardError.BaseStream.CopyToAsync($stderr, 81920, $cancel.Token)
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $capture.timed_out = $true
            $process.Kill()
            if (-not $process.WaitForExit(5000)) { throw 'Direct child did not exit after termination' }
        }
        $capture.exit_code = $process.ExitCode
        $copies = [System.Threading.Tasks.Task]::WhenAll([System.Threading.Tasks.Task[]]@($outTask, $errTask))
        if (-not $copies.Wait(5000)) { throw 'Capture pipes did not close after process exit' }
    } catch {
        $capture.runner_error = "$Id.runner-error.log"
        Save-NewText (Join-Path $script:runDirectory $capture.runner_error) $_.Exception.ToString()
        [void] $script:errors.Add('A_PROCESS_CAPTURE')
    } finally {
        $cancel.Cancel()
        if ($startedProcess -and -not $process.HasExited) {
            try {
                $process.Kill()
                if (-not $process.WaitForExit(5000)) { [void] $script:errors.Add('A_PROCESS_REAP') }
            } catch { [void] $script:errors.Add('A_PROCESS_REAP') }
        }
        if ($startedProcess -and $process.HasExited) { $capture.exit_code = $process.ExitCode }
        if ($null -ne $stdout) { $stdout.Dispose() }
        if ($null -ne $stderr) { $stderr.Dispose() }
        $cancel.Dispose()
        $process.Dispose()
    }
    return $capture
}

# Only retain small control payloads and chat cursor snapshots. Message bodies are
# counted one line at a time and never accumulated in memory or the receipt.
function Read-CliSummary([string] $Path) {
    $result = @{
        acks = [System.Collections.Generic.List[object]]::new()
        import_stats = [System.Collections.Generic.List[object]]::new()
        chats = [System.Collections.Generic.Dictionary[string, object]]::new([System.StringComparer]::Ordinal)
        messages = 0L; inboxes = 0L
        error_codes = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        warning_codes = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    }
    $reader = [System.IO.StreamReader]::new($Path, $utf8, $true)
    try {
        while ($null -ne ($line = $reader.ReadLine())) {
            $event = ConvertFrom-Json -InputObject $line -AsHashtable -Depth 100
            switch ($event.event) {
                'ack' { $result.acks.Add($event.payload) }
                'stats' { if ($event.payload.scope -eq 'import') { $result.import_stats.Add($event.payload) } }
                'chat' {
                    $chat = $event.payload
                    $result.chats.Add($chat.chat_id, [ordered]@{
                        last_ingested = $chat.last_ingested; last_analyzed = $chat.last_analyzed
                        last_reviewed = $chat.last_reviewed; unreviewed_messages = $chat.unreviewed_messages
                    })
                }
                'message' { $result.messages++ }
                'inbox' { $result.inboxes++ }
                'error' {
                    $code = if ($event.payload.code -cmatch '^E_[A-Z0-9_]+$') { $event.payload.code } else { 'E_UNRECOGNIZED' }
                    [void] $result.error_codes.Add($code)
                }
                'warning' {
                    $code = if ($event.payload.code -cmatch '^W_[A-Z0-9_]+$') { $event.payload.code } else { 'W_UNRECOGNIZED' }
                    [void] $result.warning_codes.Add($code)
                }
            }
        }
    } finally { $reader.Dispose() }
    return $result
}

function Invoke-Cli([string] $Id, [string[]] $Arguments, [int[]] $AllowedExitCodes = @(0)) {
    $argsWithProfile = @('--data-dir', $script:profile) + $Arguments
    $capture = Invoke-Captured $Id $CliPath $argsWithProfile 'cli'
    # Even failed CLI commands undergo the shared Rust/core validator. A killed
    # child may have an incomplete stream: keep that validator failure as evidence.
    if ($null -eq $capture.exit_code) { Stop-Acceptance 'A_PROCESS_NO_EXIT_CODE' }
    $check = Invoke-Captured "$Id-check" $EvalPath @(
        'check-stream', (Join-Path $script:runDirectory $capture.stdout), '--exit-code', [string] $capture.exit_code
    ) 'eval'
    Assert-Check "$Id-protocol" ($check.exit_code -eq 0 -and -not $check.timed_out -and $null -eq $check.runner_error)
    $summary = Read-CliSummary (Join-Path $script:runDirectory $capture.stdout)
    $capture['error_codes'] = @($summary.error_codes)
    $capture['warning_codes'] = @($summary.warning_codes)
    Assert-Check "$Id-exit" ($capture.exit_code -in $AllowedExitCodes -and -not $capture.timed_out -and $null -eq $capture.runner_error)
    return $summary
}

function Single-Ack($Summary, [string] $Command) {
    $acks = @($Summary.acks | Where-Object { $_.command -eq $Command })
    Assert-Check "$Command-ack" ($acks.Count -eq 1)
    return $acks[0]
}

try {
    $rootPath = Resolve-FilesystemPath $OutputRoot
    [void] [System.IO.Directory]::CreateDirectory($rootPath)
    $runId = '{0}-{1}' -f $started.ToString('yyyyMMddTHHmmssfffZ'), [Guid]::NewGuid().ToString('N')
    $newRunDirectory = Join-Path $rootPath $runId
    # Unlike CreateDirectory, New-Item without -Force refuses any existing target.
    [void] (New-Item -ItemType Directory -Path $newRunDirectory)
    $script:runDirectory = $newRunDirectory
    $script:profile = Join-Path $script:runDirectory 'profile'
    $sourcePath = Resolve-FilesystemPath $InputFile
    $CliPath = Resolve-FilesystemPath $CliPath
    $EvalPath = Resolve-FilesystemPath $EvalPath
    Assert-Check 'input-is-file' ([System.IO.File]::Exists($sourcePath))
    Assert-Check 'cli-is-file' ([System.IO.File]::Exists($CliPath))
    Assert-Check 'eval-is-file' ([System.IO.File]::Exists($EvalPath))
    $sourceBefore = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $cliHash = (Get-FileHash -LiteralPath $CliPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $evalHash = (Get-FileHash -LiteralPath $EvalPath -Algorithm SHA256).Hash.ToLowerInvariant()

    $version = Single-Ack (Invoke-Cli '01-version' @('version')) 'version'
    $required = @('version', 'config init', 'doctor', 'import', 'chats', 'messages', 'analyze', 'inbox')
    $missing = @($required | Where-Object { $_ -cnotin $version.detail.capabilities.commands })
    Assert-Check 'required-capabilities' ($missing.Count -eq 0)
    [void] (Invoke-Cli '02-config-init' @('config', 'init'))
    $doctor = Single-Ack (Invoke-Cli '03-doctor' @('doctor') @(0, 4)) 'doctor'
    Assert-Check 'doctor-offline' ($doctor.detail.remote_checked -ceq $false)
    Assert-Check 'doctor-import-ready' ($doctor.detail.readiness.import -ceq $true)

    $importArgs = @('import', $sourcePath)
    if ($SelfUid) { $importArgs += @('--self-uid', $SelfUid) }
    if ($SelfUin) { $importArgs += @('--self-uin', $SelfUin) }
    $first = Invoke-Cli '04-import-first' $importArgs
    $firstAck = Single-Ack $first 'import'
    Assert-Check 'first-import-stats' ($first.import_stats.Count -eq 1)
    $firstStats = $first.import_stats[0]
    $initial = Invoke-Cli '05-chats-before' @('chats')
    $chatCount = $initial.chats.Count
    Assert-Check 'import-chat-count' ($chatCount -eq @($firstAck.detail.chat_ids).Count -and $chatCount -eq $firstStats.chats)
    foreach ($chatId in $firstAck.detail.chat_ids) {
        Assert-Check 'import-chat-present' ($initial.chats.ContainsKey($chatId))
    }

    $repeat = Invoke-Cli '06-import-repeat' $importArgs
    $repeatAck = Single-Ack $repeat 'import'
    Assert-Check 'repeat-import-stats' ($repeat.import_stats.Count -eq 1)
    $repeatStats = $repeat.import_stats[0]
    Assert-Check 'repeat-seen' ($repeatStats.seen -eq $firstStats.seen)
    Assert-Check 'repeat-inserted-zero' ($repeatStats.inserted -eq 0)
    Assert-Check 'repeat-unchanged' ($repeatAck.changed -ceq $false)
    Assert-Check 'repeat-all-duplicate' ($repeatStats.duplicate -eq $repeatStats.seen)

    $index = 0
    foreach ($chatId in ($initial.chats.Keys | Sort-Object)) {
        $index++
        $id = '07-chat-{0:D4}' -f $index
        $messages = Invoke-Cli "$id-messages" @('messages', '--chat', $chatId)
        $messageCount += $messages.messages
        $plan = Single-Ack (Invoke-Cli "$id-plan" @('analyze', '--chat', $chatId, '--dry-run')) 'analyze'
        Assert-Check "$id-plan-unchanged" ($plan.changed -ceq $false -and $null -ne $plan.detail.plan)
        $inbox = Invoke-Cli "$id-inbox" @('inbox', '--chat', $chatId, '--all')
        Assert-Check "$id-inbox-present" ($inbox.inboxes -eq 1)
    }
    Assert-Check 'message-count' ($messageCount -eq $firstStats.inserted)
    $final = Invoke-Cli '08-chats-after' @('chats')
    Assert-Check 'chat-count-unchanged' ($final.chats.Count -eq $initial.chats.Count)
    foreach ($chatId in $initial.chats.Keys) {
        Assert-Check 'chat-still-present' ($final.chats.ContainsKey($chatId))
        foreach ($field in @('last_ingested', 'last_analyzed', 'last_reviewed', 'unreviewed_messages')) {
            Assert-Check "$field-unchanged" ($initial.chats[$chatId][$field] -ceq $final.chats[$chatId][$field])
        }
    }
    $success = $true
} catch {
    if ($script:errors.Count -eq 0) { [void] $script:errors.Add('A_SCRIPT_FAILURE') }
    if ($script:runDirectory -and [System.IO.Directory]::Exists($script:runDirectory)) {
        try { Save-NewText (Join-Path $script:runDirectory 'script-error.log') $_.ToString() } catch { }
    }
} finally {
    if ($sourceBefore) {
        try {
            $sourceAfter = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
            $sourceUnchanged = $sourceBefore -ceq $sourceAfter
        } catch { [void] $script:errors.Add('A_SOURCE_RECHECK') }
        $script:checks.Add([ordered]@{ name = 'source-unchanged'; passed = $sourceUnchanged })
        if (-not $sourceUnchanged) { [void] $script:errors.Add('A_SOURCE_CHANGED'); $success = $false }
    }
    foreach ($binary in @(
        @{ name = 'cli'; path = $CliPath; before = $cliHash },
        @{ name = 'eval'; path = $EvalPath; before = $evalHash }
    )) {
        if (-not $binary.before) { continue }
        $after = $null
        try {
            $after = (Get-FileHash -LiteralPath $binary.path -Algorithm SHA256).Hash.ToLowerInvariant()
        } catch { [void] $script:errors.Add("A_$($binary.name.ToUpperInvariant())_RECHECK") }
        if ($binary.name -eq 'cli') { $cliHashAfter = $after } else { $evalHashAfter = $after }
        $unchanged = $binary.before -ceq $after
        $script:checks.Add([ordered]@{ name = "$($binary.name)-unchanged"; passed = $unchanged })
        if (-not $unchanged) { [void] $script:errors.Add("A_$($binary.name.ToUpperInvariant())_CHANGED") }
    }
    if ($script:errors.Count -gt 0) { $success = $false }
    if ($script:runDirectory -and [System.IO.Directory]::Exists($script:runDirectory)) {
        $receipt = [ordered]@{
            version = 1; success = $success; started_at = $started.ToString('o')
            finished_at = [DateTimeOffset]::UtcNow.ToString('o')
            hashes = [ordered]@{
                source_before = $sourceBefore; source_after = $sourceAfter
                cli_before = $cliHash; cli_after = $cliHashAfter; eval_before = $evalHash; eval_after = $evalHashAfter
            }
            source_unchanged = $sourceUnchanged; model_keys_removed = $true
            chats = $chatCount; messages = $messageCount; first_import = $firstStats; repeat_import = $repeatStats
            checks = @($script:checks); commands = @($script:commands); errors = @($script:errors)
            limitations = @('cloud_calls_not_tested', 'analysis_quality_not_tested', 'export_provenance_not_verified', 'gui_not_tested')
        }
        $receiptPath = Join-Path $script:runDirectory 'receipt.json'
        try {
            Save-NewText $receiptPath ($receipt | ConvertTo-Json -Depth 30)
            $result = if ($success) { 'PASS' } else { 'FAIL' }
            Write-Output "$result chats=$chatCount messages=$messageCount checks=$($script:checks.Count) errors=$($script:errors.Count)"
            Write-Output "receipt=$receiptPath"
        } catch {
            $success = $false
            Write-Output 'FAIL A_RECEIPT_WRITE; private captures retained; receipt unavailable'
        }
    } else {
        Write-Output "FAIL codes=$(@($script:errors) -join ','); no acceptance commands executed"
    }
}
if ($success) { exit 0 }
exit 1
