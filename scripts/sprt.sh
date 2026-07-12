#!/usr/bin/env bash

set -euo pipefail

NEW="${1:-}"
OLD="${2:-}"

if [[ -z "$NEW" || -z "$OLD" ]]; then
    echo "Use ./sprt.sh <new> <old>"
    exit 1
fi

CONCURRENCY=1
LOG_FILE="./bench/logs/${NEW}_vs_${OLD}"
mkdir -p $LOG_FILE
./bench/fastchess/fastchess \
  -engine cmd=./bench/${NEW}/chrusty name=${NEW} option.Hash=32 \
  -engine cmd=./bench/${OLD}/chrusty name=${OLD} option.Hash=32 \
  -openings file=./bench/fastchess/UHO_Lichess_4852_v1.epd format=epd order=random \
  -each tc=20+0.2 \
  -resign movecount=3 score=600 -draw movenumber=40 movecount=6 score=20 \
  -sprt elo0=0 elo1=5 alpha=0.15 beta=0.15 \
  -rounds 100000 -concurrency $CONCURRENCY \
  -pgnout notation=san nodes=true file=$LOG_FILE/pgn append=false \
  -show-latency -recover -ratinginterval 1 \
  -log file=$LOG_FILE/log append=false realtime=true engine=true level=warn