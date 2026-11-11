#!/bin/sh
set -eu
test "$(sh sum.sh 2 3)" -eq 5
test "$(sh sum.sh -2 3)" -eq 1
test "$(sh sum.sh 0 0)" -eq 0
test "$(sh sum.sh 7 -4)" -eq 3
printf '{"checks":4,"passed":true}\n' > verified.json
