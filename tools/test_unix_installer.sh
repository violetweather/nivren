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
