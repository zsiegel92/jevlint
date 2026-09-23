#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
install_dir="${JEVLINT_INSTALL_DIR:-$HOME/.local/bin}"
build_dir="${CARGO_TARGET_DIR:-$project_dir/target}"
destination="$install_dir/jevlint"
schema_dir="$(dirname -- "$install_dir")/share/jevlint"

cd "$project_dir"
cargo build --locked --release
mkdir -p "$install_dir"
mkdir -p "$schema_dir"

temporary="$(mktemp "$install_dir/.jevlint.XXXXXX")"
trap 'rm -f "$temporary"' EXIT
install -m 755 "$build_dir/release/jevlint" "$temporary"
mv -f "$temporary" "$destination"
trap - EXIT

schema_temporary="$(mktemp "$schema_dir/.jevlint-schema.XXXXXX")"
trap 'rm -f "$schema_temporary"' EXIT
install -m 644 "$project_dir/schema/jevlint.schema.json" "$schema_temporary"
mv -f "$schema_temporary" "$schema_dir/jevlint.schema.json"
trap - EXIT

echo "Installed jevlint to $destination"
echo "Installed config schema to $schema_dir/jevlint.schema.json"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "Add $install_dir to PATH to run jevlint by name." >&2 ;;
esac
