#!/bin/bash
#
# Mine Litecoin regtest blocks
#
# Usage:
#   ./5c_mine_litecoin.sh [count] [wallet_name]
#   Default: 1 block, wallet = $LITECOIN_WALLET
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_env.sh"
source "$SCRIPT_DIR/_litecoin_env.sh"

COUNT="${1:-1}"
WALLET="${2:-$LITECOIN_WALLET}"

# Ensure wallet exists (ignore if it already does)
ltc_cli createwallet "$WALLET" >/dev/null 2>&1 || true

ADDR="$(ltc_cli -rpcwallet="$WALLET" getnewaddress)"

echo "Mining $COUNT block(s) to address: $ADDR"
if ! ltc_cli -rpcwallet="$WALLET" generatetoaddress "$COUNT" "$ADDR" >/dev/null 2>&1; then
  echo "ERROR: generatetoaddress failed"
  echo "Error details:"
  ltc_cli -rpcwallet="$WALLET" generatetoaddress "$COUNT" "$ADDR" 2>&1 || true
  exit 1
fi

HEIGHT="$(ltc_cli getblockcount 2>/dev/null || echo "unknown")"
echo "Current Litecoin regtest height: $HEIGHT"


