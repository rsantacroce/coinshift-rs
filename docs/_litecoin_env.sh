#!/bin/bash
#
# Shared Litecoin regtest environment for docs scripts.
# Safe to source from other scripts: `source "$(dirname "$0")/_litecoin_env.sh"`
#
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_env.sh"

# ---- Config (override any of these via env) ----
: "${RPC_USER:=user}"
: "${RPC_PASSWORD:=passwordDC}"

: "${LITECOIN_DIR:=${PROJECT_ROOT}/litecoin/build/bin}"
: "${LITECOIND:=$LITECOIN_DIR/litecoind}"
: "${LITECOIN_CLI:=$LITECOIN_DIR/litecoin-cli}"

: "${LITECOIN_RPC_PORT:=19443}"
: "${LITECOIN_P2P_PORT:=39333}"
: "${LITECOIN_DATADIR:=${PROJECT_ROOT}/coinshift-litecoin-data}"
: "${LITECOIN_WALLET:=litecoinwallet}"

maybe_resolve_litecoin_bins_from_path() {
  if [ ! -x "$LITECOIND" ] && command -v litecoind >/dev/null 2>&1; then
    LITECOIND="$(command -v litecoind)"
  fi
  if [ ! -x "$LITECOIN_CLI" ] && command -v litecoin-cli >/dev/null 2>&1; then
    LITECOIN_CLI="$(command -v litecoin-cli)"
  fi
}

ensure_litecoin_bins() {
  maybe_resolve_litecoin_bins_from_path

  if [ ! -x "$LITECOIND" ]; then
    echo "ERROR: litecoind not found or not executable: $LITECOIND" >&2
    echo "Set LITECOIN_DIR, LITECOIND, or install litecoind in PATH." >&2
    exit 1
  fi
  if [ ! -x "$LITECOIN_CLI" ]; then
    echo "ERROR: litecoin-cli not found or not executable: $LITECOIN_CLI" >&2
    echo "Set LITECOIN_DIR, LITECOIN_CLI, or install litecoin-cli in PATH." >&2
    exit 1
  fi
}

ltc_cli() {
  ensure_litecoin_bins
  "$LITECOIN_CLI" -regtest -rpcwait \
    -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$LITECOIN_RPC_PORT" \
    -datadir="$LITECOIN_DATADIR" \
    "$@"
}


