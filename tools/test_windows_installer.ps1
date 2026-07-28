[CmdletBinding()]
param(
    [string]$Installer = (Join-Path $PSScriptRoot "..\install\install.ps1")
)

$ErrorActionPreference = "Stop"
$fixture = Join-Path ([System.IO.Path]::GetTempPath()) ("nivren-windows-installer-test-" + [guid]::NewGuid())
$installRoot = Join-Path $fixture "installed"
$fakeCommands = Join-Path $fixture "commands"
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$machine = switch ($architecture) {
    "X64" { "x64" }
    "Arm64" { "arm64" }
    default { throw "unsupported Windows fixture architecture: $architecture" }
}
$releaseRoot = Join-Path $fixture "release\nivren-v2.0.0-windows-$machine"
$asset = "nivren-v2.0.0-windows-$machine.zip"
$archive = Join-Path $fixture $asset
$originalPath = $env:Path

try {
    New-Item -ItemType Directory -Force $fakeCommands, (Join-Path $releaseRoot "bin"), (Join-Path $installRoot "bin"), (Join-Path $installRoot "versions\1.0.0\bin") | Out-Null
    $program = @'
#include <stdio.h>
int main(void) {
    puts("Nivren installer fixture");
    return 0;
}
'@
    $fixtureSource = Join-Path $fixture "nivren-installer-fixture.c"
    $fixtureBinary = Join-Path $fixture "niv.exe"
    $fixtureObject = Join-Path $fixture "nivren-installer-fixture.obj"
    Set-Content -LiteralPath $fixtureSource -Value $program -NoNewline
    $compiler = (Get-Command cl.exe -ErrorAction Stop).Source
    & $compiler /nologo /O1 "/Fe:$fixtureBinary" "/Fo:$fixtureObject" $fixtureSource
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $fixtureBinary -PathType Leaf)) {
        throw "failed to build the native $machine installer fixture"
    }
    Copy-Item $fixtureBinary (Join-Path $releaseRoot "bin\niv.exe")
    Copy-Item $fixtureBinary (Join-Path $installRoot "bin\niv.exe")
    Copy-Item $fixtureBinary (Join-Path $installRoot "versions\1.0.0\bin\niv.exe")
    Set-Content -LiteralPath (Join-Path $installRoot ".nivren-install-root") -Value "nivren-managed-root-v1" -NoNewline
    Set-Content -LiteralPath (Join-Path $installRoot "current-version") -Value "1.0.0" -NoNewline

    Compress-Archive -Path $releaseRoot -DestinationPath $archive
    $digest = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath (Join-Path $fixture "SHA256SUMS") -Value "$digest  $asset`n" -NoNewline
    Set-Content -LiteralPath (Join-Path $fixture "channel.pub") -Value ("11" * 32) -NoNewline
    $assets = [ordered]@{}
    $assets[$asset] = $digest
    $manifest = [ordered]@{
        format = 1
        channel = "beta"
        version = "2.0.0"
        generation = 2
        issued_at = 1
        expires_at = 4102444800
        base_url = "https://example.invalid/v2.0.0"
        assets = $assets
        signature = ("22" * 64)
    }
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $fixture "channel-beta.json")

    Set-Content -LiteralPath (Join-Path $fakeCommands "gh.cmd") -Value "@exit /b 0`r`n" -NoNewline
    $env:Path = "$fakeCommands;$originalPath"
    $global:NivrenInstallerFixtureRoot = $fixture
    $global:NivrenInstallerFixtureAsset = $asset
    $global:NivrenInstallerFixtureArchive = $archive
    function global:Invoke-WebRequest {
        param(
            [Parameter(Position = 0)][string]$Uri,
            [string]$OutFile,
            [switch]$UseBasicParsing
        )
        $source = if ($Uri.EndsWith("channel-beta.json")) {
            Join-Path $global:NivrenInstallerFixtureRoot "channel-beta.json"
        } elseif ($Uri.EndsWith("SHA256SUMS")) {
            Join-Path $global:NivrenInstallerFixtureRoot "SHA256SUMS"
        } elseif ($Uri.EndsWith($global:NivrenInstallerFixtureAsset)) {
            $global:NivrenInstallerFixtureArchive
        } else {
            throw "unexpected installer URL: $Uri"
        }
        Copy-Item $source $OutFile -Force
    }

    & $Installer -Channel beta -ChannelKey (Join-Path $fixture "channel.pub") -InstallRoot $installRoot -Yes -NoPath -VSCode Skip
    if ((Get-Content -Raw (Join-Path $installRoot "current-version")) -ne "2.0.0") { throw "channel install did not activate version 2.0.0" }
    if ((Get-Content -Raw (Join-Path $installRoot "previous-version")) -ne "1.0.0") { throw "channel install did not preserve version 1.0.0" }
    if ((Get-Content -Raw (Join-Path $installRoot "channel-beta-generation")) -ne "2") { throw "channel generation was not retained" }
    if ((Get-Content -Raw (Join-Path $installRoot "current-channel")) -ne "beta") { throw "channel identity was not retained" }
    if ((Get-Content -Raw (Join-Path $installRoot "channel-public-key")) -ne ("11" * 32)) { throw "channel key was not retained" }
    $receipt = Get-Content -Raw (Join-Path $installRoot "install-receipt.json") | ConvertFrom-Json
    if ($receipt.version -ne "2.0.0" -or $receipt.previous -ne "1.0.0") { throw "channel install receipt is incorrect" }

    $manifest.assets[$asset] = ("00" * 32)
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $fixture "channel-beta.json")
    $digestRejected = $false
    try {
        & $Installer -Channel beta -InstallRoot $installRoot -Yes -NoPath -VSCode Skip
    } catch {
        $digestRejected = $_.Exception.Message -match "Signed channel digest verification failed"
    }
    if (-not $digestRejected) { throw "installer accepted a channel/archive digest mismatch" }
    if ((Get-Content -Raw (Join-Path $installRoot "current-version")) -ne "2.0.0") { throw "failed update changed the active version" }

    & $Installer -Rollback -InstallRoot $installRoot
    if ((Get-Content -Raw (Join-Path $installRoot "current-version")) -ne "1.0.0") { throw "rollback did not restore version 1.0.0" }
    & $Installer -Uninstall -InstallRoot $installRoot
    if (Test-Path $installRoot) { throw "uninstall left the managed root behind" }
} finally {
    Remove-Item function:\Invoke-WebRequest -ErrorAction SilentlyContinue
    Remove-Variable NivrenInstallerFixtureRoot -Scope Global -ErrorAction SilentlyContinue
    Remove-Variable NivrenInstallerFixtureAsset -Scope Global -ErrorAction SilentlyContinue
    Remove-Variable NivrenInstallerFixtureArchive -Scope Global -ErrorAction SilentlyContinue
    $env:Path = $originalPath
    if (Test-Path $fixture) { Remove-Item -Recurse -Force $fixture }
}
