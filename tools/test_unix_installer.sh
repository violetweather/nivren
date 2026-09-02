#!/bin/sh
set -eu

root=$(mktemp -d "${TMPDIR:-/tmp}/nivren-installer-test.XXXXXX")
trap 'rm -rf "$root"' EXIT HUP INT TERM
install_root="$root/install"
bin_dir="$root/bin"
mkdir -p "$install_root/versions/1.0.0/bin" "$install_root/versions/2.0.0/bin" "$bin_dir"

for version in 1.0.0 2.0.0; do
  binary="$install_root/versions/$version/bin/niv"
  printf '#!/bin/sh\nprintf '\''Nivren %s\\n'\'' '\''%s'\''\n' "$version" "$version" > "$binary"
  chmod +x "$binary"
done

printf 'nivren-managed-root-v1\n' > "$install_root/.nivren-install-root"
printf '2.0.0\n' > "$install_root/current-version"
printf '1.0.0\n' > "$install_root/previous-version"
ln -s "$install_root/versions/2.0.0/bin/niv" "$bin_dir/niv"

sh install/install.sh --rollback --install-root "$install_root" --bin-dir "$bin_dir"
test "$(cat "$install_root/current-version")" = "1.0.0"
test "$(cat "$install_root/previous-version")" = "2.0.0"
test "$(readlink "$bin_dir/niv")" = "$install_root/versions/1.0.0/bin/niv"
grep '"version":"1.0.0"' "$install_root/install-receipt.json" >/dev/null

sh install/install.sh --rollback --install-root "$install_root" --bin-dir "$bin_dir"
test "$(cat "$install_root/current-version")" = "2.0.0"
test "$(cat "$install_root/previous-version")" = "1.0.0"

sh install/install.sh --uninstall --install-root "$install_root" --bin-dir "$bin_dir"
test ! -e "$install_root"
test ! -e "$bin_dir/niv"

channel_root="$root/channel-install"
channel_bin="$root/channel-bin"
fixture="$root/fixture"
fakebin="$root/fakebin"
mkdir -p "$channel_root/versions/1.0.0/bin" "$channel_bin" "$fixture/release/nivren-v2.0.0" "$fakebin"
printf '#!/bin/sh\nif [ "$1 $2" = "release verify-channel" ]; then exit 0; fi\nprintf "Nivren 1.0.0\\n"\n' > "$channel_root/versions/1.0.0/bin/niv"
chmod +x "$channel_root/versions/1.0.0/bin/niv"
ln -s "$channel_root/versions/1.0.0/bin/niv" "$channel_bin/niv"
printf 'nivren-managed-root-v1\n' > "$channel_root/.nivren-install-root"
printf '1.0.0\n' > "$channel_root/current-version"
printf 'trusted-test-key\n' > "$fixture/channel.pub"

os=$(uname -s)
arch=$(uname -m)
case "$os" in Darwin) platform=macos ;; Linux) platform=linux ;; *) exit 69 ;; esac
case "$arch" in x86_64|amd64) machine=x64 ;; arm64|aarch64) machine=arm64 ;; *) exit 69 ;; esac
asset="nivren-v2.0.0-$platform-$machine.zip"
archive_root="$fixture/release/nivren-v2.0.0-$platform-$machine"
mkdir -p "$archive_root/bin"
printf '#!/bin/sh\nprintf "Nivren 2.0.0\\n"\n' > "$archive_root/bin/niv"
chmod +x "$archive_root/bin/niv"
(cd "$fixture/release" && zip -qr "$fixture/$asset" "nivren-v2.0.0-$platform-$machine")
digest=$(shasum -a 256 "$fixture/$asset" | awk '{print $1}')
printf '%s  %s\n' "$digest" "$asset" > "$fixture/SHA256SUMS"
printf '{\n  "format": 1,\n  "channel": "beta",\n  "version": "2.0.0",\n  "generation": 2,\n  "issued_at": 1,\n  "expires_at": 4102444800,\n  "base_url": "https://example.invalid/v2.0.0",\n  "assets": {\n    "%s": "%s"\n  },\n  "signature": "test"\n}\n' "$asset" "$digest" > "$fixture/channel-beta.json"
# The installer consumes the verifier's stdout rather than the manifest, so
# the stub speaks the real CLI's output: the verified line and one asset
# line per offered archive.
printf '#!/bin/sh\nif [ "$1 $2" = "release verify-channel" ]; then printf "verified beta 2.0.0 generation 2\\nasset %s %s\\n"; exit 0; fi\nprintf "Nivren 1.0.0\\n"\n' "$asset" "$digest" > "$channel_root/versions/1.0.0/bin/niv"

printf '#!/bin/sh\nset -eu\noutput=""\nurl=""\nwhile [ "$#" -gt 0 ]; do\n  case "$1" in --output) output=$2; shift 2 ;; http*) url=$1; shift ;; *) shift ;; esac\ndone\ncase "$url" in *channel-beta.json) cp "$FIXTURE_DIR/channel-beta.json" "$output" ;; *SHA256SUMS) cp "$FIXTURE_DIR/SHA256SUMS" "$output" ;; *.zip) cp "$FIXTURE_DIR/'"$asset"'" "$output" ;; *) exit 22 ;; esac\n' > "$fakebin/curl"
printf '#!/bin/sh\nexit 0\n' > "$fakebin/gh"
chmod +x "$fakebin/curl" "$fakebin/gh"

FIXTURE_DIR="$fixture" PATH="$fakebin:$PATH" sh install/install.sh --channel beta --channel-key "$fixture/channel.pub" --install-root "$channel_root" --bin-dir "$channel_bin" --yes --no-path --no-vscode
test "$(cat "$channel_root/current-version")" = "2.0.0"
test "$(cat "$channel_root/channel-beta-generation")" = "2"
test "$(cat "$channel_root/current-channel")" = "beta"
test "$(cat "$channel_root/channel-public-key")" = "trusted-test-key"
grep '"version":"2.0.0"' "$channel_root/install-receipt.json" >/dev/null
