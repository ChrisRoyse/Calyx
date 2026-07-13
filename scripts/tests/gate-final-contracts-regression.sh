#!/usr/bin/env bash
# Source-only final lock, runner, safe-removal, and target-clean contracts.
[[ "${BASH_SOURCE[0]}" != "$0" ]] || {
  echo "source this module from check-tmpdir-regression.sh" >&2
  exit 64
}

# shellcheck source=gate-lock-wait-regression.sh
source "$lock_wait_scenario"
# shellcheck source=gate-runner-contract-regression.sh
source "$runner_contracts"
echo
echo "=================================================================="
echo "SAFE REMOVAL CONTRACTS - FD identity, symlink, swap, and recovery"
echo "=================================================================="
python3 "$safe_helper_regression" "$safe_helper_src" "$work/safe-rmtree"
# shellcheck source=check-cleanup-prediagnostic-regression.sh
source "$cleanup_prediagnostic_contracts"
# shellcheck source=cargo-target-clean-regression.sh
source "$target_clean_contracts"
# shellcheck source=gate-check-final-cleanup-regression.sh
source "$check_final_cleanup_contracts"
