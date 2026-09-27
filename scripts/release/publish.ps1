#Requires -Version 7.2
param(
    [Parameter(Mandatory)][string] $Version,
    [Parameter(Mandatory)][string] $Commit,
    [Parameter(Mandatory)][string] $Image,
    [string] $Directory = 'dist'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$tag = "v$Version"
$sourceImage = "chat-tldr-ci:$Commit"
$files = @("chat-tldr-$Version-windows-x86_64.zip", "chat-tldr-gui-$Version-windows-x86_64.zip", "chat-tldr-$Version-linux-x86_64-musl.tar.gz", 'docker-image.tar')
$sums = foreach ($name in $files) {
    $path = Join-Path $Directory $name
    $expected = (Get-Content -LiteralPath "$path.sha256" -Raw).Trim()
    $actual = "$((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant())  $name"
    if ($actual -cne $expected) { throw "Checksum mismatch: $name" }
    $actual
}
$sums | Set-Content -LiteralPath (Join-Path $Directory 'SHA256SUMS') -Encoding utf8
docker load --input (Join-Path $Directory 'docker-image.tar')
if ($LASTEXITCODE -ne 0) { throw 'Cannot load the tested Docker artifact.' }
$revision = docker image inspect $sourceImage --format '{{index .Config.Labels "org.opencontainers.image.revision"}}'
if ($LASTEXITCODE -ne 0 -or $revision -cne $Commit) { throw 'Docker artifact revision does not match the release.' }
$notes = Join-Path $Directory 'release-notes.md'
@"
CLI + Windows GUI + Docker release, built from commit $Commit.

- Windows x64: extract the ZIP and run chat-tldr.exe.
- Windows GUI: extract the GUI ZIP and keep chat-tldr-gui.exe, chat-tldr.exe and chat-tldr-qce-manager.exe together; run chat-tldr-gui.exe. QQ acquisition connects to an existing local QCE/NapCat service (not bundled).
- Model settings: configure OpenAI/Anthropic endpoints, model names and API keys in the GUI or CLI. Jev uses SystemOne. Saved keys are plaintext in the local config and take precedence over environment variables; configuration queries and logs do not echo them.
- Linux x64: static musl executable in the tar.gz archive.
- Docker: ``docker pull ${Image}:$Version`` (linux/amd64, non-root scratch image).
- Offline Docker: download docker-image.tar, verify SHA256SUMS, then ``docker load -i docker-image.tar``. Loaded image: ``$sourceImage``.

See docs/DOCKER.md and docs/RELEASING.md for persistent data, configuration, scope and validation limits. Chat text is sent unredacted to configured cloud models. Cloud model quality is not certified by the synthetic release checks.
GUI CI includes Linux native rendering on Xvfb/Mesa and Windows optimized protocol/pointer tests plus archive round-trip checks; it does not certify every desktop GPU, display scale or native file picker.
"@ | Set-Content -LiteralPath $notes -Encoding utf8
$assets = @($files | ForEach-Object { Join-Path $Directory $_ }) + (Join-Path $Directory 'SHA256SUMS')
# Create a draft first; a failure must not produce a public, incomplete Release.
# An existing release is deliberately not overwritten by a retry.
gh release create $tag @assets --draft --verify-tag --title "chat-tldr $tag" --notes-file $notes
if ($LASTEXITCODE -ne 0) { throw 'Cannot create release draft; inspect any existing draft before retrying.' }
docker tag $sourceImage "${Image}:$Version"
if ($LASTEXITCODE -ne 0) { throw 'Cannot tag the tested image.' }
docker push "${Image}:$Version"
if ($LASTEXITCODE -ne 0) { throw 'Versioned image push failed; release remains a draft.' }
docker tag $sourceImage "${Image}:latest"
if ($LASTEXITCODE -ne 0) { throw 'Cannot tag latest.' }
docker push "${Image}:latest"
if ($LASTEXITCODE -ne 0) { throw 'Latest image push failed; release remains a draft.' }
gh release edit $tag --draft=false --latest
if ($LASTEXITCODE -ne 0) { throw 'Images are published, but the GitHub release remains a draft.' }
