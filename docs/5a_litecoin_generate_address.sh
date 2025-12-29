#!/bin/bash
#
# Generate a new Litecoin regtest address (prints to stdout)
#
# Usage:
#   ./5a_litecoin_generate_address.sh [wallet_name]
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_env.sh"
source "$SCRIPT_DIR/_litecoin_env.sh"

WALLET="${1:-$LITECOIN_WALLET}"

# Ensure wallet exists (ignore if it already does)
ltc_cli createwallet "$WALLET" >/dev/null 2>&1 || true

ltc_cli -rpcwallet="$WALLET" getnewaddress


