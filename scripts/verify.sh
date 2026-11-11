#!/usr/bin/env bash
set -euo pipefail
cargo test --workspace --locked
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel 2
ctest --test-dir build --output-on-failure
cargo run --locked -q -- classify examples/classification.json
cargo run --locked -q -- regression examples/regression.json
cargo run --locked -q -- plan configs/cluster.yaml
if cargo run --locked -q -- gate configs/gates.json; then
  echo 'ERROR: unmeasured release gates incorrectly passed' >&2
  exit 1
fi
