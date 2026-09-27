# Windows and macOS matrices use this registry. All workspace modules run on
# Windows for every change; macOS adds native coverage for the application edges.
param(
    [ValidateSet('all', 'macos')]
    [string]$Platform = 'all'
)

$ErrorActionPreference = 'Stop'

$modules = @(
    @{ module = 'core'; package = 'chat-tldr-core' }
    @{ module = 'qce'; package = 'chat-tldr-qce' }
    @{ module = 'engine'; package = 'chat-tldr-engine' }
    @{ module = 'cli'; package = 'chat-tldr' }
    @{ module = 'gui'; package = 'chat-tldr-gui' }
    @{ module = 'qce-manager'; package = 'chat-tldr-qce-manager' }
    @{ module = 'eval'; package = 'chat-tldr-eval' }
)

$manifest = Join-Path $PSScriptRoot '../../Cargo.toml'
$metadataJson = & cargo metadata --manifest-path $manifest --format-version 1 --no-deps --locked
if ($LASTEXITCODE -ne 0) {
    throw 'Cargo metadata failed; refusing to produce an incomplete CI matrix.'
}
$metadata = $metadataJson | ConvertFrom-Json -ErrorAction Stop
$actual = @($metadata.packages |
    Where-Object { $_.id -in $metadata.workspace_members } |
    ForEach-Object { $_.name } |
    Sort-Object)
$expected = @($modules | ForEach-Object { $_.package } | Sort-Object)
if ($actual.Count -ne $expected.Count -or (Compare-Object $expected $actual)) {
    throw 'Workspace membership changed; update scripts/ci/modules.ps1 to cover every package.'
}

if ($Platform -eq 'macos') {
    $nativeModules = @('cli', 'gui', 'eval', 'qce-manager')
    $modules = @($modules | Where-Object { $_.module -in $nativeModules })
    if ($modules.Count -ne $nativeModules.Count) {
        throw 'macOS coverage is incomplete; refusing to produce a partial CI matrix.'
    }
}

ConvertTo-Json -InputObject @{ include = $modules } -Depth 3 -Compress
