#!/usr/bin/env bash

# Based on https://github.com/consensus-shipyard/lotus/blob/main/scripts/ipc/src/r31415926-single-validator.sh

# There is also https://github.com/consensus-shipyard/lotus/blob/main/scripts/ipc/src/subnet-validator.sh
# but it looks incomplete at the moment, not starting the validator.

set -e

eudico wait-api --timeout=300s
sleep 5

# Do not fail if the keys have already been imported,
# which would be the case if this container was restarted.
set +e

echo "[*] Importing wallet key"
eudico wallet import --as-default --format=json-lotus /wallet.key

echo "[*] Initializing validator config"
eudico mir validator config init

# Find an address where the validator can be reached by others. This is going into some kind of membership file
# which should be shared with the other validators, which isn't done, there is no shared volume for that.
# The process seems to have some `--membership=onchain` parameter that might help.
validator_addr=`eudico mir validator config validator-addr | grep -vE '(/ip6/)' | grep -v "127.0.0.1"  | grep -E '/tcp/1347'`

if [ "$IPC_SUBNET_ID" != "/r31415926" ]; then
  # On subnets Mir crashes if the address contains the <wallet-id>@ part.
  validator_addr=$(echo $validator_addr | sed 's/^.*@//')
fi

echo "[*] Adding validator with address $validator_addr"
eudico mir validator config add-validator $validator_addr

set -e

echo "[*] Starting validator"

if [ "$IPC_SUBNET_ID" == "/r31415926" ]; then
  eudico mir validator run --nosync
else
  # In the infra scripts this is called in mine-subnet.sh
  # It's not clear if there is any problem launching this before the validator has joined the subnet.
  eudico mir validator run --nosync --membership onchain --ipcagent-url=http://${AGENT_HOSTNAME}:3030/json_rpc
fi
