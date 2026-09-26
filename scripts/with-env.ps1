#requires -Version 7.2
<#
.SYNOPSIS
从本地 .env 向单个子进程提供模型密钥，不修改当前 PowerShell 的环境。
.EXAMPLE
./scripts/with-env.ps1 cargo run -p chat-tldr -- doctor
.EXAMPLE
./scripts/with-env.ps1 ./target/debug/chat-tldr.exe doctor
#>
$ErrorActionPreference = 'Stop'
# Avoid PowerShell parameter binding consuming the child's --, -Command or common parameters.
$arguments = @($args)
$EnvFile = Join-Path $PSScriptRoot '../.env'
if ($arguments.Count -gt 0 -and $arguments[0] -ceq '-EnvFile') {
    if ($arguments.Count -lt 3) {
        [Console]::Error.WriteLine('Usage: with-env.ps1 [-EnvFile <path>] <application> [arguments...]')
        exit 2
    }
    $EnvFile = [string]$arguments[1]
    $arguments = @($arguments | Select-Object -Skip 2)
}
if ($arguments.Count -eq 0 -or [string]::IsNullOrWhiteSpace([string]$arguments[0])) {
    [Console]::Error.WriteLine('Usage: with-env.ps1 [-EnvFile <path>] <application> [arguments...]')
    exit 2
}
$Command = [string]$arguments[0]
$CommandArguments = @($arguments | Select-Object -Skip 1)

function Read-ModelEnvironment([string] $Path) {
    $allowed = @('TYPESAFE_API_KEY', 'CHAT_TLDR_LLM_API_KEY')
    $values = [Collections.Generic.Dictionary[string, string]]::new([StringComparer]::Ordinal)
    $lines = [IO.File]::ReadAllLines($Path, [Text.UTF8Encoding]::new($false, $true))
    for ($index = 0; $index -lt $lines.Length; $index++) {
        $line = $lines[$index]
        if ($line -match '^\s*(?:#.*)?$') { continue }
        $invalid = "Invalid or unsupported .env entry at line $($index + 1)."
        if ($line -notmatch '^\s*(?:export[ \t]+)?([A-Za-z_][A-Za-z0-9_]*)[ \t]*=[ \t]*(.*)$') {
            throw $invalid
        }
        $key = $Matches[1]
        $raw = $Matches[2].Trim()
        if ($key -cnotin $allowed -or $values.ContainsKey($key)) { throw $invalid }
        if ($raw.StartsWith('"') -or $raw.StartsWith("'")) {
            $quote = $raw[0]
            $end = $raw.IndexOf($quote, 1)
            if ($end -lt 0) { throw $invalid }
            $tail = $raw.Substring($end + 1).Trim()
            if ($tail.Length -gt 0 -and -not $tail.StartsWith('#')) { throw $invalid }
            $value = $raw.Substring(1, $end - 1)
        } else {
            # A hash inside a bare value is literal; whitespace before it begins a comment.
            $value = ($raw -replace '(?:^|\s+)#.*$', '').TrimEnd()
            if ($value.Contains('"') -or $value.Contains("'")) { throw $invalid }
        }
        if ($value.Contains([char]0)) { throw $invalid }
        $values.Add($key, $value)
    }
    # Return the dictionary as one object, including when it is empty.
    return ,$values
}

try {
    $resolved = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($EnvFile)
    $values = Read-ModelEnvironment $resolved
} catch {
    # Never include source lines, exception data or key values in diagnostics.
    [Console]::Error.WriteLine('Cannot load .env: use the documented model key names and valid single-line assignments.')
    exit 2
}

try {
    $application = Get-Command -Name $Command -CommandType Application -ErrorAction Stop | Select-Object -First 1
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $application.Source
    $start.UseShellExecute = $false
    foreach ($argument in $CommandArguments) { $start.ArgumentList.Add($argument) }
    foreach ($entry in $values.GetEnumerator()) {
        # A variable already present in the parent environment wins, including an empty value.
        if (-not $start.Environment.ContainsKey($entry.Key)) {
            $start.Environment[$entry.Key] = $entry.Value
        }
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw 'Child process did not start.' }
        $process.WaitForExit()
        $code = $process.ExitCode
    } finally {
        $process.Dispose()
    }
} catch {
    [Console]::Error.WriteLine('Cannot run the requested application with the local model environment.')
    exit 2
}
exit $code
