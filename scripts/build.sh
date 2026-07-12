#!/usr/bin/env bash

set -euo pipefail

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
    echo "Use ./build.sh <version>"
    exit 1
fi

cargo build -r
mkdir -p ./bench/$VERSION
cp ./target/release/chrusty ./bench/$VERSION/chrusty