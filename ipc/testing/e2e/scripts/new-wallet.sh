#!/usr/bin/env bash

# Create a new wallet and export the address and key.
# Call it on the node where the wallet will be created.

set -e

if [ $# -ne 5 ]
then
    echo "usage: ./new-wallet.sh <agent-dir> <node-dir> <wallet-dir> <ipc-agent> <ipc-agent-url>"
    exit 1
fi

AGENT_DIR=$1
NODE_DIR=$2
WALLET_DIR=$3
AGENT=$4
AGENT_URL=$5

source $NODE_DIR/.env
source $AGENT_DIR/.env

if [ "${IPC_NODE_TYPE}" == "eudico" ]; then
  DAEMON_ID=ipc-node-${IPC_NODE_NR}-daemon

  echo "[*] Creating new wallet with agent-$IPC_AGENT_NR on $IPC_NODE_TYPE node-$IPC_NODE_NR in subnet $IPC_SUBNET_ID"

  # Example output from the agent:
  # [2023-04-14T14:24:27Z INFO  ipc_agent::cli::commands::wallet::new] created new wallet with address WalletNewResponse { address: "t1qn46gmcao6vnujtim7l2a4ombut2ywyhh4ccdga" } in subnet "/r31415926"
  set +e
  LOG=$($AGENT wallet new --ipc-agent-url $AGENT_URL --key-type secp256k1 --subnet=$IPC_SUBNET_ID 2>&1)
  STATUS=$?
  if [ $STATUS != 0 ]; then
    echo $LOG
    exit 1
  fi
  set -e
  ADDR=$(echo $LOG | sed 's/^.*address: "\([^"]*\)".*$/\1/')

  if [ -z "$ADDR" ]; then
    echo "ERROR: Could not find the address in the logs.";
    exit 1
  fi

  echo "[*] Exporting the key for address $ADDR"
  WALLET_KEY=$(docker exec -it $DAEMON_ID eudico wallet export --lotus-json $ADDR)

  echo "[*] Writing the wallet key and address to $WALLET_DIR"
  mkdir -p $WALLET_DIR
  echo $ADDR > $WALLET_DIR/address
  echo $WALLET_KEY > $WALLET_DIR/wallet.key

else
  echo "Don't know how to make a wallet for node type: $IPC_NODE_TYPE";
  exit 1;
fi
