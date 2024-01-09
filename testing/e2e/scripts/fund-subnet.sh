#!/usr/bin/env bash

# Send funds into a subnet.
# Call it on the child subnet node, so we can figure out from .env what the subnet ID is.

set -e

if [ $# -ne 4 ]
then
    echo "usage: ./fund-subnet.sh <agent-dir> <node-dir> <ipc-agent> <ipc-agent-url>"
    exit 1
fi

AGENT_DIR=$1
NODE_DIR=$2
AGENT=$3
AGENT_URL=$4

# Rest of the variables from env vars. Before sourcing more .env
SUBNET_FUNDS=${IPC_SUBNET_FUNDS:-0}

source $NODE_DIR/.env
source $AGENT_DIR/.env

IPC_WALLET_DIR=$(dirname $IPC_WALLET_KEY)
ADDR=$(cat $IPC_WALLET_DIR/address)

run() {
  CMD=$@
  STATUS=0
  # This command often fails for the first time for some reason.
  set +e
  n=0
  until [ "$n" -ge 3 ]
  do
    echo $CMD
    $CMD
    STATUS=$?
    if [ $STATUS = 0 ]; then
      break;
    fi
    echo "[*] Failed; retrying a bit later"
    n=$((n+1))
    sleep 10
  done
  set -e
  if [ $STATUS != 0 ]; then
    exit $STATUS;
  fi
}

if [ "$SUBNET_FUNDS" != "0" ]; then
  echo "[*] Funding $IPC_SUBNET_ID ($IPC_SUBNET_NAME) by wallet-$IPC_WALLET_NR ($ADDR) with $SUBNET_FUNDS token(s) using agent-$IPC_AGENT_NR"
  run $AGENT cross-msg fund --ipc-agent-url $AGENT_URL --subnet $IPC_SUBNET_ID --from $ADDR $SUBNET_FUNDS
else
  echo "[*] Fund amount is zero; skip sneding funds to $ADDR in $IPC_SUBNET_ID"
fi
