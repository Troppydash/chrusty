# Chrusty Chess

### Prereq
- Fastchess is from [https://github.com/Disservin/fastchess]. Paste the binary in `./bench/fastchess/fastchess`.
- Openbook is from [https://github.com/official-stockfish/books]. Paste it in `./bench/fastchess/..book`.

### Build
For development uci, use
```bash
cargo run
# or
cargo run -r
```

To build a version for sprt, run
```bash
./scripts/build.sh 1.0.0
```

To test sprt between two versions, run
```bash
./scripts/sprt.sh 1.0.1 1.0.0
```
The sprt log files are in `./bench/logs/<new>_vs_<old>`.
