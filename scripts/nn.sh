#!/usr/bin/env bash

set -e

if [ -z "$1" ]; then
    echo "Use ./nn.sh <path/to/file.bin>"
    exit 1
fi

FILE_PATH="$1"

if [ ! -f "$FILE_PATH" ]; then
    echo "Error: File '$FILE_PATH' does not exist."
    exit 1
fi

if [ ! -f "./hash.nn" ]; then
    echo "Error: File hash.nn not found at script root."
    exit 1
fi

DIR_NAME=$(dirname "$FILE_PATH")
HASH=$(sha256sum "$FILE_PATH" | cut -c1-14)

NEW_FILE_NAME="${HASH}.bin"
NEW_FILE_PATH="${DIR_NAME}/${NEW_FILE_NAME}"

echo "Renamed: $FILE_PATH -> $NEW_FILE_PATH"
mv "$FILE_PATH" "$NEW_FILE_PATH"

echo $HASH > ./hash.nn