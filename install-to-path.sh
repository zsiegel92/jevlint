#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
install_dir="${JEVLINT_INSTALL_DIR:-$HOME/.local/bin}"
build_dir="${CARGO_TARGET_DIR:-$project_dir/target}"
destination="$install_dir/jevlint"

cd "$project_dir"
cargo build --locked --release
mkdir -p "$install_dir"

temporary="$(mktemp "$install_dir/.jevlint.XXXXXX")"
trap 'rm -f "$temporary"' EXIT
install -m 755 "$build_dir/release/jevlint" "$temporary"
mv -f "$temporary" "$destination"
trap - EXIT

echo "Installed jevlint to $destination"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "Add $install_dir to PATH to run jevlint by name." >&2 ;;
esac
