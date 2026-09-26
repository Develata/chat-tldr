#Requires -Version 7.2
param([switch] $Publish, [string] $Tag, [string] $Commit, [string] $Repository = 'Develata/chat-tldr')
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$manifest = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
$section = [regex]::Match($manifest, '(?ms)^\[workspace\.package\]\s*(.*?)(?=^\[|\z)').Groups[1].Value
$version = [regex]::Match($section, '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"').Groups[1].Value
if (-not $version) { throw 'Expected a stable workspace version (X.Y.Z).' }
if ($Publish) {
    if ($Tag -cne "v$version") { throw "Tag must match Cargo.toml: v$version" }
    if ($Commit -notmatch '^[0-9a-f]{40}$') { throw 'Missing exact release commit.' }
    $tagCommit = git rev-parse "$Tag^{commit}"
    if ($LASTEXITCODE -ne 0 -or $tagCommit -cne $Commit) { throw 'Tag does not resolve to the checked-out commit.' }
    git merge-base --is-ancestor $Commit origin/main
    if ($LASTEXITCODE -ne 0) { throw 'Release commit must already belong to origin/main.' }
}
$result = [ordered]@{ version = $version; image = "ghcr.io/$($Repository.ToLowerInvariant())"; publish = $Publish.IsPresent.ToString().ToLowerInvariant() }
if ($env:GITHUB_OUTPUT) {
    foreach ($entry in $result.GetEnumerator()) { "$($entry.Key)=$($entry.Value)" | Out-File $env:GITHUB_OUTPUT -Encoding utf8 -Append }
}
$result | ConvertTo-Json -Compress
