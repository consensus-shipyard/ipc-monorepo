#!/usr/bin/env bash

# Join a subnet as a validator. Call it on the $IPC_NODE_NR where we want to run the subnet,
# so that we can pick up the wallet, the subnet IDs and the validator net address, then
# execute the request on the parent. The child node needs to run at this point for the
# validator address to be available.

set -e

if [ $# -ne 4 ]
then
    echo "usage: ./join-subnet.sh <agent-dir> <node-dir> <ipc-agent> <ipc-agent-url>"
    exit 1
fi

AGENT_DIR=$1
NODE_DIR=$2
AGENT=$3
AGENT_URL=$4

# Rest of the variables from env vars. Before sourcing more .env
COLLATERAL=${IPC_COLLATERAL:-0}

source $NODE_DIR/.env
source $AGENT_DIR/.env


IPC_WALLET_DIR=$(dirname $IPC_WALLET_KEY)
ADDR=$(cat $IPC_WALLET_DIR/address)

DAEMON_ID=ipc-node-${IPC_NODE_NR}-daemon

echo "[*] Waiting for the daemon to start"
docker exec -it $DAEMON_ID eudico wait-api --timeout 350s
sleep 10

echo "[*] Get the validator net address"
set +e
# On subnets the validator address should not contain the identity part, just the /ip4/... onwards.
VALIDATOR_NET_ADDR=$(docker exec -it $DAEMON_ID eudico mir validator config validator-addr \
                    | grep -vE '(/ip6/)' | grep -v "127.0.0.1"  | grep -E '/tcp/1347' | sed 's/^.*@//' | tr -d '\r' | tr -d '\n' )
STATUS=$?
set -e
if [ $STATUS != 0 ]; then
  echo $VALIDATOR_NET_ADDR
  exit $STATUS
fi

echo "[*] Validator net address: $VALIDATOR_NET_ADDR"

run() {
  echo $@
  $@
}

if [ "$COLLATERAL" != "0" ]; then
  echo "[*] Joining $IPC_SUBNET_ID ($IPC_SUBNET_NAME) by wallet-$IPC_WALLET_NR ($ADDR) with $COLLATERAL token(s) using agent-$IPC_AGENT_NR"
  run $AGENT subnet join --ipc-agent-url $AGENT_URL --subnet $IPC_SUBNET_ID --from $ADDR --collateral $COLLATERAL --validator-net-addr $VALIDATOR_NET_ADDR
else
  echo "[*] Collateral amount is zero; skip joining by $ADDR"
fi
