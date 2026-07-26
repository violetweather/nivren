# Nivren guided installers

The installers detect the operating system and CPU, download the matching official archive, verify its SHA-256 checksum, optionally verify its GitHub build attestation when GitHub CLI is available, retain the bundled documentation, install a stable `niv` command, and optionally configure PATH and VS Code.

Interactive macOS/Linux install:

```sh
curl --proto '=https' --tlsv1.2 -fsSLO https://raw.githubusercontent.com/violetweather/nivren/main/install/install.sh
sh install.sh
```

Unattended macOS/Linux install with recommended choices:

```sh
sh install.sh --yes
```

Interactive Windows install from PowerShell:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/violetweather/nivren/main/install/install.ps1 -OutFile install.ps1
Set-ExecutionPolicy -Scope Process Bypass
.\install.ps1
```

Unattended Windows install with recommended choices:

```powershell
.\install.ps1 -Yes
```

Use `--help` on macOS/Linux or `Get-Help .\install.ps1 -Detailed` on Windows for automation controls. The manual archives remain available for users who do not want an installer to update their profile or user PATH.
