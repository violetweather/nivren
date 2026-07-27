[CmdletBinding()]
param(
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]*$')]
    [string]$Version = "0.10.0-beta.6",
    [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA "Nivren"),
    [switch]$Uninstall,
    [switch]$Yes,
    [switch]$NoPath,
    [ValidateSet("Ask", "Install", "Skip")]
    [string]$VSCode = "Ask"
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Confirm-Choice([string]$Prompt, [bool]$Default) {
    if ($Yes) { return $Default }
    $answer = Read-Host $Prompt
    if ([string]::IsNullOrWhiteSpace($answer)) { return $Default }
    return $answer -match '^(y|yes)$'
}

$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$machine = switch ($architecture) {
    "X64" { "x64" }
    "Arm64" { "arm64" }
    default { throw "Unsupported Windows architecture: $architecture" }
}

$asset = "nivren-v$Version-windows-$machine.zip"
$base = "https://github.com/violetweather/nivren/releases/download/v$Version"
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ("nivren-install-" + [guid]::NewGuid())
$versionRoot = Join-Path $InstallRoot "versions\$Version"
$binDir = Join-Path $InstallRoot "bin"

if ($Uninstall) {
    if ([string]::IsNullOrWhiteSpace($InstallRoot)) { throw "Refusing an empty install root" }
    if (-not (Test-Path $InstallRoot -PathType Container)) { throw "Installation root does not exist: $InstallRoot" }
    $rootItem = Get-Item -LiteralPath $InstallRoot -Force
    if ($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { throw "Refusing a reparse-point install root: $InstallRoot" }
    $marker = Join-Path $InstallRoot ".nivren-install-root"
    if (-not (Test-Path $marker -PathType Leaf)) { throw "Refusing to remove an installation without $marker" }
    if ((Get-Content -Raw $marker).Trim() -ne "nivren-managed-root-v1") { throw "Installation ownership marker is invalid" }
    $resolvedRoot = [System.IO.Path]::GetFullPath($InstallRoot).TrimEnd('\')
    $homeRoot = [System.IO.Path]::GetFullPath($HOME).TrimEnd('\')
    $driveRoot = [System.IO.Path]::GetPathRoot($resolvedRoot).TrimEnd('\')
    if ($resolvedRoot -eq $homeRoot -or $resolvedRoot -eq $driveRoot) { throw "Refusing unsafe install root: $InstallRoot" }
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $pathParts = @($userPath -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ine $binDir.TrimEnd('\') })
    [Environment]::SetEnvironmentVariable("Path", ($pathParts -join ';'), "User")
    Remove-Item -Recurse -Force $InstallRoot
    Write-Host "Nivren was uninstalled." -ForegroundColor Cyan
    return
}

Write-Host "Nivren $Version installer" -ForegroundColor Cyan
Write-Host "Platform: windows-$machine"
Write-Host "Install:  $versionRoot"
Write-Host "Command:  $binDir\niv.exe"

New-Item -ItemType Directory -Force $temporary | Out-Null
try {
    $archive = Join-Path $temporary $asset
    $checksums = Join-Path $temporary "SHA256SUMS"
    Invoke-WebRequest -UseBasicParsing "$base/$asset" -OutFile $archive
    Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile $checksums

    $line = Get-Content $checksums | Where-Object { $_ -match "\s$([regex]::Escape($asset))$" } | Select-Object -First 1
    if (-not $line) { throw "Release checksum is missing for $asset" }
    $expected = ($line -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "Checksum verification failed" }

    $gh = Get-Command gh -ErrorAction SilentlyContinue
    if ($gh) {
        & $gh.Source attestation verify --repo violetweather/nivren $archive | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "GitHub build provenance verification failed" }
        Write-Host "Verified checksum and GitHub build provenance." -ForegroundColor Green
    } else {
        Write-Host "Verified SHA-256 checksum. Install GitHub CLI to verify build provenance automatically." -ForegroundColor Yellow
    }

    $unpacked = Join-Path $temporary "unpacked"
    Expand-Archive $archive -DestinationPath $unpacked
    $sourceRoot = Join-Path $unpacked "nivren-v$Version-windows-$machine"
    $sourceBinary = Join-Path $sourceRoot "bin\niv.exe"
    if (-not (Test-Path $sourceBinary -PathType Leaf)) { throw "Release archive has an unexpected layout" }

    New-Item -ItemType Directory -Force (Split-Path $versionRoot), $binDir | Out-Null
    $staging = "$versionRoot.new.$PID"
    if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
    Copy-Item -Recurse $sourceRoot $staging
    if (Test-Path $versionRoot) { Remove-Item -Recurse -Force $versionRoot }
    Move-Item $staging $versionRoot
    Copy-Item (Join-Path $versionRoot "bin\niv.exe") (Join-Path $binDir "niv.exe") -Force
    Set-Content -LiteralPath (Join-Path $InstallRoot "current-version") -Value $Version -NoNewline
    Set-Content -LiteralPath (Join-Path $InstallRoot ".nivren-install-root") -Value "nivren-managed-root-v1" -NoNewline

    $addPath = -not $NoPath
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $pathParts = @($userPath -split ';' | Where-Object { $_ })
    $alreadyPresent = $pathParts | Where-Object { $_.TrimEnd('\') -ieq $binDir.TrimEnd('\') }
    if ($addPath -and -not $alreadyPresent) {
        $addPath = Confirm-Choice "Add Nivren to your user PATH? [Y/n]" $true
    }
    if ($addPath -and -not $alreadyPresent) {
        $newPath = (@($pathParts) + $binDir) -join ';'
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Write-Host "Updated your user PATH." -ForegroundColor Green
    }
    if (($env:Path -split ';') -notcontains $binDir) { $env:Path = "$binDir;$env:Path" }

    $code = Get-Command code -ErrorAction SilentlyContinue
    $installVSCode = $VSCode -eq "Install" -or ($VSCode -eq "Ask" -and $code -and (Confirm-Choice "Install the Nivren VS Code extension? [Y/n]" $true))
    if ($installVSCode) {
        if (-not $code) { Write-Warning "VS Code's 'code' command is unavailable; skipping extension." }
        else {
            $extension = "nivren-$Version.vsix"
            $extensionPath = Join-Path $temporary $extension
            Invoke-WebRequest -UseBasicParsing "$base/$extension" -OutFile $extensionPath
            $extensionLine = Get-Content $checksums | Where-Object { $_ -match "\s$([regex]::Escape($extension))$" } | Select-Object -First 1
            if (-not $extensionLine) { throw "Release checksum is missing for $extension" }
            $extensionExpected = ($extensionLine -split '\s+')[0].ToLowerInvariant()
            $extensionActual = (Get-FileHash $extensionPath -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($extensionActual -ne $extensionExpected) { throw "VS Code extension checksum verification failed" }
            if ($gh) {
                & $gh.Source attestation verify --repo violetweather/nivren $extensionPath | Out-Null
                if ($LASTEXITCODE -ne 0) { throw "VS Code extension provenance verification failed" }
            }
            & $code.Source --install-extension $extensionPath --force
            if ($LASTEXITCODE -ne 0) { throw "VS Code extension installation failed" }
        }
    }

    & (Join-Path $binDir "niv.exe") version
    Write-Host "Nivren is installed. Open a new terminal, then run: niv help" -ForegroundColor Cyan
} finally {
    if (Test-Path $temporary) { Remove-Item -Recurse -Force $temporary }
}
