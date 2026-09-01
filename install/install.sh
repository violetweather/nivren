#!/bin/sh
set -eu

VERSION="1.0.1"
INSTALL_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}/nivren"
BIN_DIR="$HOME/.local/bin"
ADD_PATH=ask
VSCODE=ask
ASSUME_YES=0
UNINSTALL=0
ROLLBACK=0
CHANNEL=""
CHANNEL_KEY=""

usage() {
  cat <<'EOF'
Nivren installer

Usage: install.sh [options]
  --version VERSION       Install a specific release (default: 1.0.1)
  --channel CHANNEL       Update from stable, beta, or nightly using a signed manifest
  --channel-key PATH      Trust this separately obtained Ed25519 channel public key
  --uninstall             Remove a Nivren installation owned by this installer
  --rollback              Switch back to the previously verified installed version
  --install-root PATH     Keep versions and documentation here
  --bin-dir PATH          Put the stable niv command here
  --yes                   Accept recommended choices without prompting
  --no-path               Do not update shell PATH configuration
  --vscode                Install the VS Code extension when `code` is available
  --no-vscode             Skip the VS Code extension
  --help                  Show this help
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) [ "$#" -ge 2 ] || { echo "missing value for --version" >&2; exit 64; }; VERSION=$2; shift 2 ;;
    --channel) [ "$#" -ge 2 ] || { echo "missing value for --channel" >&2; exit 64; }; CHANNEL=$2; shift 2 ;;
    --channel-key) [ "$#" -ge 2 ] || { echo "missing value for --channel-key" >&2; exit 64; }; CHANNEL_KEY=$2; shift 2 ;;
    --uninstall) UNINSTALL=1; shift ;;
    --rollback) ROLLBACK=1; shift ;;
    --install-root) [ "$#" -ge 2 ] || { echo "missing value for --install-root" >&2; exit 64; }; INSTALL_ROOT=$2; shift 2 ;;
    --bin-dir) [ "$#" -ge 2 ] || { echo "missing value for --bin-dir" >&2; exit 64; }; BIN_DIR=$2; shift 2 ;;
    --yes) ASSUME_YES=1; shift ;;
    --no-path) ADD_PATH=no; shift ;;
    --vscode) VSCODE=yes; shift ;;
    --no-vscode) VSCODE=no; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 64 ;;
  esac
done

case "$VERSION" in
  ""|[!A-Za-z0-9]*|*[!A-Za-z0-9._-]*) echo "invalid version: $VERSION" >&2; exit 64 ;;
esac
case "$INSTALL_ROOT$BIN_DIR" in *"
"*|*"'"*|*'"'*) echo "install paths cannot contain newlines or quotes" >&2; exit 64 ;; esac

[ "$UNINSTALL" -eq 0 ] || [ "$ROLLBACK" -eq 0 ] || { echo "--uninstall and --rollback cannot be combined" >&2; exit 64; }
case "$CHANNEL" in ""|stable|beta|nightly) ;; *) echo "invalid channel: $CHANNEL" >&2; exit 64 ;; esac

if [ "$UNINSTALL" -eq 1 ]; then
  [ -n "$INSTALL_ROOT" ] || { echo "refusing an empty install root" >&2; exit 65; }
  [ ! -L "$INSTALL_ROOT" ] || { echo "refusing a symbolic-link install root: $INSTALL_ROOT" >&2; exit 65; }
  [ -d "$INSTALL_ROOT" ] || { echo "installation root does not exist: $INSTALL_ROOT" >&2; exit 65; }
  resolved_root=$(CDPATH= cd -- "$INSTALL_ROOT" && pwd -P)
  resolved_home=$(CDPATH= cd -- "$HOME" && pwd -P)
  case "$resolved_root" in ""|/|"$resolved_home") echo "refusing unsafe install root: $resolved_root" >&2; exit 65 ;; esac
  marker="$INSTALL_ROOT/.nivren-install-root"
  [ -f "$marker" ] || { echo "refusing to remove an installation without $marker" >&2; exit 65; }
  [ "$(cat "$marker")" = "nivren-managed-root-v1" ] || { echo "installation ownership marker is invalid" >&2; exit 65; }
  if [ -L "$BIN_DIR/niv" ]; then
    link_target=$(readlink "$BIN_DIR/niv")
    case "$link_target" in "$INSTALL_ROOT"/versions/*/bin/niv) rm -f "$BIN_DIR/niv" ;; *) echo "leaving unrelated $BIN_DIR/niv in place" >&2 ;; esac
  fi
  if [ -f "$INSTALL_ROOT/path-profile" ]; then
    profile=$(cat "$INSTALL_ROOT/path-profile")
    if [ -f "$profile" ]; then
      cleaned=$(mktemp "${TMPDIR:-/tmp}/nivren-profile.XXXXXX")
      awk 'skip { skip = 0; next } $0 == "# Nivren" { skip = 1; next } { print }' "$profile" > "$cleaned"
      mv "$cleaned" "$profile"
    fi
  fi
  rm -rf "$resolved_root"
  echo "Nivren was uninstalled."
  exit 0
fi

if [ "$ROLLBACK" -eq 1 ]; then
  marker="$INSTALL_ROOT/.nivren-install-root"
  [ -f "$marker" ] || { echo "installation ownership marker is missing" >&2; exit 65; }
  [ "$(cat "$marker")" = "nivren-managed-root-v1" ] || { echo "installation ownership marker is invalid" >&2; exit 65; }
  [ -f "$INSTALL_ROOT/current-version" ] || { echo "current version receipt is missing" >&2; exit 65; }
  [ -f "$INSTALL_ROOT/previous-version" ] || { echo "no previous Nivren version is available" >&2; exit 65; }
  current=$(cat "$INSTALL_ROOT/current-version")
  previous=$(cat "$INSTALL_ROOT/previous-version")
  case "$current" in ""|*[!A-Za-z0-9._-]*) echo "current version receipt is invalid" >&2; exit 65 ;; esac
  case "$previous" in ""|*[!A-Za-z0-9._-]*) echo "previous version receipt is invalid" >&2; exit 65 ;; esac
  previous_binary="$INSTALL_ROOT/versions/$previous/bin/niv"
  [ -x "$previous_binary" ] || { echo "previous Nivren binary is missing: $previous_binary" >&2; exit 65; }
  mkdir -p "$BIN_DIR"
  ln -sfn "$previous_binary" "$BIN_DIR/niv"
  printf '%s\n' "$previous" > "$INSTALL_ROOT/current-version"
  printf '%s\n' "$current" > "$INSTALL_ROOT/previous-version"
  printf '{"format":1,"version":"%s","previous":"%s","platform":"local-rollback"}\n' "$previous" "$current" > "$INSTALL_ROOT/install-receipt.json"
  "$BIN_DIR/niv" version
  echo "Rolled back Nivren from $current to $previous."
  exit 0
fi

if [ "$ASSUME_YES" -eq 1 ]; then
  [ "$ADD_PATH" = ask ] && ADD_PATH=yes
  [ "$VSCODE" = ask ] && { command -v code >/dev/null 2>&1 && VSCODE=yes || VSCODE=no; }
fi

ask_yes_no() {
  prompt=$1
  default=$2
  if [ ! -t 0 ]; then printf '%s' "$default"; return; fi
  printf '%s ' "$prompt" >&2
  IFS= read -r answer
  case "$answer" in y|Y|yes|YES) printf yes ;; n|N|no|NO) printf no ;; *) printf '%s' "$default" ;; esac
}

os=$(uname -s)
arch=$(uname -m)
case "$os" in Darwin) platform=macos ;; Linux) platform=linux ;; *) echo "unsupported operating system: $os" >&2; exit 69 ;; esac
case "$arch" in x86_64|amd64) machine=x64 ;; arm64|aarch64) machine=arm64 ;; *) echo "unsupported architecture: $arch" >&2; exit 69 ;; esac

temporary=$(mktemp -d "${TMPDIR:-/tmp}/nivren-install.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 69; }
command -v unzip >/dev/null 2>&1 || { echo "unzip is required" >&2; exit 69; }

channel_generation=""
channel_digest=""
if [ -n "$CHANNEL" ]; then
  verifier="$BIN_DIR/niv"
  [ -x "$verifier" ] || { echo "signed channel updates require an existing verified Nivren install; use --version for the first install" >&2; exit 65; }
  [ -n "$CHANNEL_KEY" ] || CHANNEL_KEY="$INSTALL_ROOT/channel-public-key"
  [ -f "$CHANNEL_KEY" ] || { echo "channel public key is missing; pass --channel-key from a separately trusted source" >&2; exit 65; }
  manifest="$temporary/$CHANNEL.json"
  curl --fail --location --proto '=https' --tlsv1.2 --output "$manifest" "https://violetweather.github.io/nivren-site/channel-$CHANNEL.json"
  minimum=0
  [ ! -f "$INSTALL_ROOT/channel-$CHANNEL-generation" ] || minimum=$(cat "$INSTALL_ROOT/channel-$CHANNEL-generation")
  case "$minimum" in ""|*[!0-9]*) echo "stored channel generation is invalid" >&2; exit 65 ;; esac
  now=$(date +%s)
  "$verifier" release verify-channel "$manifest" "$CHANNEL_KEY" "$now" "$minimum"
  VERSION=$(awk -F'"' '/^[[:space:]]*"version"[[:space:]]*:/ { print $4; exit }' "$manifest")
  channel_generation=$(awk '/^[[:space:]]*"generation"[[:space:]]*:/ { value=$2; gsub(/,/, "", value); print value; exit }' "$manifest")
  case "$VERSION" in ""|[!A-Za-z0-9]*|*[!A-Za-z0-9._-]*) echo "signed channel version is invalid" >&2; exit 65 ;; esac
  case "$channel_generation" in ""|*[!0-9]*) echo "signed channel generation is invalid" >&2; exit 65 ;; esac
fi

asset="nivren-v${VERSION}-${platform}-${machine}.zip"
base="https://github.com/violetweather/nivren/releases/download/v${VERSION}"
if [ -n "$CHANNEL" ]; then
  channel_digest=$(awk -F'"' -v name="$asset" '$2 == name { print $4; exit }' "$manifest")
  [ -n "$channel_digest" ] || { echo "signed channel does not offer $asset" >&2; exit 65; }
fi

echo "Nivren ${VERSION} installer"
echo "Platform: ${platform}-${machine}"
echo "Install:  ${INSTALL_ROOT}/versions/${VERSION}"
echo "Command:  ${BIN_DIR}/niv"

curl --fail --location --proto '=https' --tlsv1.2 --output "$temporary/$asset" "$base/$asset"
curl --fail --location --proto '=https' --tlsv1.2 --output "$temporary/SHA256SUMS" "$base/SHA256SUMS"
expected=$(awk -v name="$asset" '$2 == name { print $1 }' "$temporary/SHA256SUMS")
[ -n "$expected" ] || { echo "release checksum is missing for $asset" >&2; exit 65; }
if command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "$temporary/$asset" | awk '{print $1}')
elif command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$temporary/$asset" | awk '{print $1}')
else
  echo "shasum or sha256sum is required" >&2
  exit 69
fi
[ "$actual" = "$expected" ] || { echo "checksum verification failed" >&2; exit 65; }
[ -z "$channel_digest" ] || [ "$actual" = "$channel_digest" ] || { echo "signed channel digest verification failed" >&2; exit 65; }

if command -v gh >/dev/null 2>&1; then
  gh attestation verify --repo violetweather/nivren "$temporary/$asset" >/dev/null
  echo "Verified checksum and GitHub build provenance."
else
  echo "Verified SHA-256 checksum. Install GitHub CLI to verify build provenance automatically."
fi

unzip -q "$temporary/$asset" -d "$temporary/unpacked"
source_root="$temporary/unpacked/nivren-v${VERSION}-${platform}-${machine}"
[ -x "$source_root/bin/niv" ] || { echo "release archive has an unexpected layout" >&2; exit 65; }

version_root="$INSTALL_ROOT/versions/$VERSION"
mkdir -p "$INSTALL_ROOT/versions" "$BIN_DIR"
previous_version=""
if [ -f "$INSTALL_ROOT/current-version" ]; then
  previous_version=$(cat "$INSTALL_ROOT/current-version")
  case "$previous_version" in ""|*[!A-Za-z0-9._-]*) echo "current version receipt is invalid" >&2; exit 65 ;; esac
fi
staging="$INSTALL_ROOT/versions/.${VERSION}.new.$$"
rm -rf "$staging"
cp -R "$source_root" "$staging"
rm -rf "$version_root"
mv "$staging" "$version_root"
ln -sfn "$version_root/bin/niv" "$BIN_DIR/niv"
printf '%s\n' "$VERSION" > "$INSTALL_ROOT/current-version"
if [ -n "$previous_version" ] && [ "$previous_version" != "$VERSION" ]; then
  printf '%s\n' "$previous_version" > "$INSTALL_ROOT/previous-version"
fi
printf '%s\n' "nivren-managed-root-v1" > "$INSTALL_ROOT/.nivren-install-root"
printf '{"format":1,"version":"%s","previous":"%s","platform":"%s-%s","bin_dir":"%s"}\n' "$VERSION" "$previous_version" "$platform" "$machine" "$BIN_DIR" > "$INSTALL_ROOT/install-receipt.json"
if [ -n "$CHANNEL" ]; then
  printf '%s\n' "$channel_generation" > "$INSTALL_ROOT/channel-$CHANNEL-generation"
  [ "$CHANNEL_KEY" = "$INSTALL_ROOT/channel-public-key" ] || cp "$CHANNEL_KEY" "$INSTALL_ROOT/channel-public-key"
  printf '%s\n' "$CHANNEL" > "$INSTALL_ROOT/current-channel"
fi

if [ "$ADD_PATH" = ask ]; then
  case ":$PATH:" in *":$BIN_DIR:"*) ADD_PATH=no ;; *) ADD_PATH=$(ask_yes_no "Add $BIN_DIR to your PATH? [Y/n]" yes) ;; esac
fi
if [ "$ADD_PATH" = yes ]; then
  shell_name=$(basename "${SHELL:-sh}")
  case "$shell_name" in
    zsh) profile="$HOME/.zprofile" ;;
    bash) profile="$HOME/.bash_profile"; [ -e "$profile" ] || profile="$HOME/.bashrc" ;;
    fish) profile="${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish"; mkdir -p "$(dirname "$profile")" ;;
    *) profile="$HOME/.profile" ;;
  esac
  marker="# Nivren"
  if ! grep -F "$marker" "$profile" >/dev/null 2>&1; then
    if [ "$shell_name" = fish ]; then
      printf '\n%s\nfish_add_path '\''%s'\''\n' "$marker" "$BIN_DIR" >> "$profile"
    else
      printf '\n%s\nexport PATH='\''%s'\'':"$PATH"\n' "$marker" "$BIN_DIR" >> "$profile"
    fi
    printf '%s\n' "$profile" > "$INSTALL_ROOT/path-profile"
  fi
  PATH="$BIN_DIR:$PATH"
  export PATH
  echo "Updated PATH in $profile"
fi

if [ "$VSCODE" = ask ] && command -v code >/dev/null 2>&1; then
  VSCODE=$(ask_yes_no "Install the Nivren VS Code extension? [Y/n]" yes)
fi
if [ "$VSCODE" = yes ]; then
  command -v code >/dev/null 2>&1 || { echo "VS Code's 'code' command is unavailable; skipping extension." >&2; VSCODE=no; }
fi
if [ "$VSCODE" = yes ]; then
  extension="nivren-${VERSION}.vsix"
  curl --fail --location --proto '=https' --tlsv1.2 --output "$temporary/$extension" "$base/$extension"
  extension_expected=$(awk -v name="$extension" '$2 == name { print $1 }' "$temporary/SHA256SUMS")
  [ -n "$extension_expected" ] || { echo "release checksum is missing for $extension" >&2; exit 65; }
  if command -v shasum >/dev/null 2>&1; then
    extension_actual=$(shasum -a 256 "$temporary/$extension" | awk '{print $1}')
  else
    extension_actual=$(sha256sum "$temporary/$extension" | awk '{print $1}')
  fi
  [ "$extension_actual" = "$extension_expected" ] || { echo "VS Code extension checksum verification failed" >&2; exit 65; }
  if command -v gh >/dev/null 2>&1; then
    gh attestation verify --repo violetweather/nivren "$temporary/$extension" >/dev/null
  fi
  code --install-extension "$temporary/$extension" --force
fi

"$BIN_DIR/niv" version
echo "Nivren is installed. Open a new terminal, then run: niv help"
