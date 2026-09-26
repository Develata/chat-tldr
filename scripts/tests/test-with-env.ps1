#requires -Version 7.2
$ErrorActionPreference = 'Stop'
$wrapper = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../with-env.ps1'))
$pwsh = (Get-Command pwsh -CommandType Application | Select-Object -First 1).Source
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testDirectory = Join-Path $tempBase ('chat-tldr-env-test-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testDirectory)
$testDirectory = [IO.Path]::GetFullPath($testDirectory)
$envPath = Join-Path $testDirectory 'synthetic env file.env'
$child = Join-Path $testDirectory 'synthetic child.ps1'
$checks = 0

function Assert-Condition([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw "FAIL: $Message" }
    $script:checks++
}

function Invoke-Wrapper([string[]] $ChildArguments = @(), [hashtable] $ParentValues = @{}) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $pwsh
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($key in @('TYPESAFE_API_KEY', 'CHAT_TLDR_LLM_API_KEY')) {
        [void]$start.Environment.Remove($key)
    }
    foreach ($entry in $ParentValues.GetEnumerator()) { $start.Environment[$entry.Key] = $entry.Value }
    foreach ($argument in @('-NoProfile', '-File', $wrapper, '-EnvFile', $envPath, $pwsh, '-NoProfile', '-File', $child) + $ChildArguments) {
        $start.ArgumentList.Add($argument)
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        [void]$process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(20000)) {
            $process.Kill($true)
            throw 'Synthetic child timed out.'
        }
        return [pscustomobject]@{ Code = $process.ExitCode; Out = $stdout.GetAwaiter().GetResult(); Err = $stderr.GetAwaiter().GetResult() }
    } finally { $process.Dispose() }
}

try {
    # The child emits synthetic environment values only; the wrapper itself must stay silent.
    $childText = @'
$ErrorActionPreference = 'Stop'
[pscustomobject]@{
    jev = [Environment]::GetEnvironmentVariable('TYPESAFE_API_KEY')
    llm = [Environment]::GetEnvironmentVariable('CHAT_TLDR_LLM_API_KEY')
    argv = @($args)
} | ConvertTo-Json -Compress
[Console]::Error.WriteLine('synthetic-child-stderr')
exit 7
'@
    [IO.File]::WriteAllText($child, $childText, [Text.UTF8Encoding]::new($false))
    $envText = @'
# Synthetic values; never use real model keys in this test.
export TYPESAFE_API_KEY = 'synthetic-je v #literal' # trailing comment
CHAT_TLDR_LLM_API_KEY = "synthetic-$(throw nope)-$env:PATH"
'@
    [IO.File]::WriteAllText($envPath, $envText.Replace("`n", "`r`n"), [Text.UTF8Encoding]::new($true))
    $before = (Get-FileHash -LiteralPath $envPath).Hash
    $arguments = @('argument with spaces', 'literal"quote', '--', '-p', '中文', '')
    $result = Invoke-Wrapper -ChildArguments $arguments
    Assert-Condition ($result.Code -eq 7) "child exit code is preserved (observed $($result.Code); stderr: $($result.Err))"
    $body = $result.Out | ConvertFrom-Json
    Assert-Condition ($body.jev -ceq 'synthetic-je v #literal') 'BOM, CRLF, export, single quotes and comments are parsed'
    Assert-Condition ($body.llm -ceq 'synthetic-$(throw nope)-$env:PATH') 'double-quoted values are literal data'
    Assert-Condition (($body.argv | ConvertTo-Json -Compress) -ceq ($arguments | ConvertTo-Json -Compress)) 'argument boundaries and empty values are preserved'
    Assert-Condition ($result.Err.Trim() -ceq 'synthetic-child-stderr') 'stderr is inherited without wrapper output'
    Assert-Condition ((Get-FileHash -LiteralPath $envPath).Hash -ceq $before) 'environment source remains unchanged'

    $result = Invoke-Wrapper -ParentValues @{ TYPESAFE_API_KEY = 'synthetic-parent-wins' }
    $body = $result.Out | ConvertFrom-Json
    Assert-Condition ($body.jev -ceq 'synthetic-parent-wins') 'parent environment takes priority'
    $result = Invoke-Wrapper -ParentValues @{ TYPESAFE_API_KEY = '' }
    $body = $result.Out | ConvertFrom-Json
    Assert-Condition ([string]::IsNullOrEmpty($body.jev)) 'explicitly empty parent environment also takes priority'

    [IO.File]::WriteAllText($envPath, "TYPESAFE_API_KEY=synthetic#literal # comment`nCHAT_TLDR_LLM_API_KEY= # empty`n", [Text.UTF8Encoding]::new($false))
    $result = Invoke-Wrapper
    $body = $result.Out | ConvertFrom-Json
    Assert-Condition ($body.jev -ceq 'synthetic#literal') 'bare internal hash stays literal'
    Assert-Condition ([string]::IsNullOrEmpty($body.llm)) 'empty assignments stay empty'

    foreach ($invalidText in @(
        'PATH=synthetic-forbidden-value',
        'UNEXPECTED_KEY=synthetic-forbidden-value',
        'synthetic-secret-without-assignment',
        'TYPESAFE_API_KEY="synthetic-unclosed',
        'TYPESAFE_API_KEY="synthetic-closed" garbage',
        "TYPESAFE_API_KEY=synthetic-first`nTYPESAFE_API_KEY=synthetic-duplicate"
    )) {
        [IO.File]::WriteAllText($envPath, $invalidText, [Text.UTF8Encoding]::new($false))
        $result = Invoke-Wrapper
        Assert-Condition ($result.Code -eq 2 -and [string]::IsNullOrEmpty($result.Out) -and $result.Err -notmatch 'synthetic') 'invalid entries fail without executing the child or leaking input'
    }
    [IO.File]::Move($envPath, (Join-Path $testDirectory 'retained-fixture.env'))
    $result = Invoke-Wrapper
    Assert-Condition ($result.Code -eq 2 -and [string]::IsNullOrEmpty($result.Out)) 'missing environment file fails before executing the child'
    Write-Output "PASS: $checks local environment checks; no cloud calls."
} finally {
    # Resolve and verify the exact test directory before recursive cleanup.
    $resolvedTestDirectory = [IO.Path]::GetFullPath($testDirectory)
    $resolvedTempBase = $tempBase.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if ($resolvedTestDirectory.StartsWith($resolvedTempBase, [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($resolvedTestDirectory) -like 'chat-tldr-env-test-*') {
        Remove-Item -LiteralPath $resolvedTestDirectory -Recurse -Force
    } else {
        throw 'Refusing to clean a test directory outside the temporary base.'
    }
}
