#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --locked --bin jevlint --bin jevlint-smoke
exec target/debug/jevlint-smoke "$@"
