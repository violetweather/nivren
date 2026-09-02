# Nivren guided installers

The installers detect the operating system and CPU, download the matching official archive, verify its SHA-256 checksum, optionally verify its GitHub build attestation when GitHub CLI is available, retain the bundled documentation, install a stable `niv` command, and optionally configure PATH and VS Code. Each archive also contains shared/static embedding libraries, `nivren.h`, an SPDX SBOM, and dependency notices for native application developers.

Interactive macOS/Linux install:

```sh
curl --proto '=https' --tlsv1.2 -fsSLO https://raw.githubusercontent.com/violetweather/nivren/v1.0.1/install/install.sh
sh install.sh
```

Unattended macOS/Linux install with recommended choices:

```sh
sh install.sh --yes
```

Interactive Windows install from PowerShell:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/violetweather/nivren/v1.0.1/install/install.ps1 -OutFile install.ps1
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

## Signed channel updates

After the first version-pinned install, opt into a stable, beta, or nightly channel with a public key obtained separately from the download host:

```sh
sh install.sh --channel beta --channel-key ./nivren-channel.pub
```

```powershell
.\install.ps1 -Channel beta -ChannelKey .\nivren-channel.pub
```

The already verified Nivren binary authenticates the channel manifest before the installer trusts its version or asset digest. Manifests expire and carry a monotonically increasing generation; the installer records the highest accepted generation and rejects rollback. It then requires the archive to match both the signed channel digest and the release checksum manifest. The trusted public key is retained for later channel updates. A first install deliberately cannot bootstrap from a channel manifest because an untrusted downloaded verifier cannot establish its own trust; use an explicit version and verify its published provenance first.

The release matrix exercises the complete signed-channel state flow on both Windows architectures and the supported Unix runners: retained keys and generations, dual digest checking, failure atomicity, local rollback, and ownership-checked uninstall. Cryptographic signature, expiry, and generation rejection are tested independently by the release-channel verifier before installer fixtures are allowed to rely on it.

Remove an installer-managed copy safely:

```sh
sh install.sh --uninstall
```

```powershell
.\install.ps1 -Uninstall
```

Uninstall only removes a directory carrying Nivren's ownership marker. It also removes the stable command and the exact PATH entry created by the installer, while refusing home directories, filesystem roots, symbolic links, and Windows reparse points. Uninstall removes all locally retained versions and receipts; copy anything needed for incident analysis first.

Use `--help` on macOS/Linux or `Get-Help .\install.ps1 -Detailed` on Windows for automation controls. The manual archives remain available for users who do not want an installer to update their profile or user PATH.
