#!/bin/bash
#
# Check Litecoin regtest wallet balance
#
# Usage:
#   ./5b_litecoin_check_balance.sh [wallet_name]
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_env.sh"
source "$SCRIPT_DIR/_litecoin_env.sh"

WALLET="${1:-$LITECOIN_WALLET}"

# Ensure wallet exists (ignore if it already does)
ltc_cli createwallet "$WALLET" >/dev/null 2>&1 || true

echo "Wallet: $WALLET"
echo -n "Balance: "
ltc_cli -rpcwallet="$WALLET" getbalance


