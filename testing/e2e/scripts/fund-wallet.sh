#!/usr/bin/env bash

# Fund an existing wallet with funds from the default wallet of the agent.
# Call it on the node where the wallet will receive the funds.

set -e

if [ $# -ne 5 ]
then
    echo "usage: ./fund-wallet.sh <agent-dir> <node-dir> <wallet-dir> <ipc-agent> <ipc-agent-url>"
    exit 1
fi

AGENT_DIR=$1
NODE_DIR=$2
WALLET_DIR=$3
AGENT=$4
AGENT_URL=$5

# Rest of the variables from env vars. Before sourcing more .env
WALLET_FUNDS=${IPC_WALLET_FUNDS:-0}

ADDR=$(cat $WALLET_DIR/address)

source $NODE_DIR/.env
source $AGENT_DIR/.env

run() {
  echo $@
  $@
}

if [ "$WALLET_FUNDS" != "0" ]; then
  echo "[*] Funding wallet-$IPC_WALLET_NR ($ADDR) with $WALLET_FUNDS token(s) using agent-$IPC_AGENT_NR on $IPC_NODE_TYPE node-$IPC_NODE_NR under $IPC_SUBNET_ID ($IPC_SUBNET_NAME)"
  run $AGENT subnet send-value --ipc-agent-url $AGENT_URL --subnet $IPC_SUBNET_ID --to $ADDR $WALLET_FUNDS
else
  echo "[*] Fund amount is zero; skip funding $ADDR"
fi
