#!/usr/bin/env bash
set -euo pipefail

extension_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
code_command="${CODE_BIN:-code}"

cd "$extension_dir"
pnpm install --frozen-lockfile
pnpm run package
"$code_command" --install-extension jevlint-vscode.vsix --force
