# Nivren guided installers

The installers detect the operating system and CPU, download the matching official archive, verify its SHA-256 checksum, optionally verify its GitHub build attestation when GitHub CLI is available, retain the bundled documentation, install a stable `niv` command, and optionally configure PATH and VS Code. Each archive also contains shared/static embedding libraries, `nivren.h`, an SPDX SBOM, and dependency notices for native application developers.

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

Every successful install writes `install-receipt.json`, retains the previous verified version, and keeps the stable command pointed at the active version. Roll back without downloading or deleting either version:

```sh
sh install.sh --rollback
```

```powershell
.\install.ps1 -Rollback
```

Rollback swaps the current and previous receipts, so repeating it restores the version that was replaced. It refuses missing ownership markers, malformed receipts, and absent binaries. Installing an explicit version remains available through `--version VERSION` or `-Version VERSION`.

Remove an installer-managed copy safely:

```sh
sh install.sh --uninstall
```

```powershell
.\install.ps1 -Uninstall
```

Uninstall only removes a directory carrying Nivren's ownership marker. It also removes the stable command and the exact PATH entry created by the installer, while refusing home directories, filesystem roots, symbolic links, and Windows reparse points. Uninstall removes all locally retained versions and receipts; copy anything needed for incident analysis first.

Use `--help` on macOS/Linux or `Get-Help .\install.ps1 -Detailed` on Windows for automation controls. The manual archives remain available for users who do not want an installer to update their profile or user PATH.
