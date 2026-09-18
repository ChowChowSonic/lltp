#!/usr/bin/env bash
#
# Pre-push checks: mirror .github/workflows/ci.yml locally.
#
# Usage:
#   ./.github/scripts/pre-push.sh              # run every check
#   ./.github/scripts/pre-push.sh fmt          # run a single check
#   ./.github/scripts/pre-push.sh clippy test  # run a subset
#   ./.github/scripts/pre-push.sh all          # every check (default)
#
# Exit status is non-zero on the first failing check.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

ALL_CHECKS=(fmt clippy test examples)

# --- helpers -----------------------------------------------------------------

info() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m    ok: %s\033[0m\n' "$*"; }
die()  { printf '\033[1;31m    error: %s\033[0m\n' "$*" >&2; exit 1; }

# --- preflight: llvm-sys 221 / inkwell 'llvm22-1' need LLVM 22 -----------------

check_llvm() {
  local llvm_config="" version=""
  if command -v llvm-config >/dev/null 2>&1; then
    llvm_config="$(command -v llvm-config)"
  elif [[ -n "${LLVM_SYS_221_PREFIX:-}" && -x "$LLVM_SYS_221_PREFIX/bin/llvm-config" ]]; then
    llvm_config="$LLVM_SYS_221_PREFIX/bin/llvm-config"
  else
    die "LLVM 22 not found. Install LLVM 22 and put llvm-config on PATH, or set LLVM_SYS_221_PREFIX."
  fi
  version="$("$llvm_config" --version 2>/dev/null || true)"
  [[ "$version" == 22* ]] || die "expected LLVM 22, found '${version:-<unknown>}' at $llvm_config."
  info "preflight: LLVM $version ($llvm_config)"
}

# --- individual checks ----------------------------------------------------------

check_fmt() {
  info "cargo fmt --check"
  cargo fmt --check
  ok "fmt"
}

check_clippy() {
  info "cargo clippy --all-targets -- -D warnings"
  cargo clippy --all-targets -- -D warnings
  ok "clippy"
}

check_test() {
  info "cargo test --all-targets"
  cargo test --all-targets
  ok "test"
}

check_examples() {
  info "cargo run --example ingest"
  cargo run --example ingest
  info "cargo run --example lowering"
  cargo run --example lowering
  ok "examples"
}

# --- dispatch -------------------------------------------------------------------

if [[ $# -eq 0 || ($# -eq 1 && "$1" == "all") ]]; then
  requested=("${ALL_CHECKS[@]}")
else
  requested=("$@")
fi

# fmt is pure rustfmt and needs no LLVM; the rest link against it.
need_llvm=false
for check in "${requested[@]}"; do
  case "$check" in clippy|test|examples) need_llvm=true ;; esac
done
if $need_llvm; then
  check_llvm
fi

for check in "${requested[@]}"; do
  case "$check" in
    fmt)      check_fmt ;;
    clippy)   check_clippy ;;
    test)     check_test ;;
    examples) check_examples ;;
    *)        die "unknown check '$check' (valid: ${ALL_CHECKS[*]})" ;;
  esac
done

printf '\n\033[1;32m✓ all pre-push checks passed\033[0m\n'