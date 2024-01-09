#!/usr/bin/env bash

set -e

echo "[*] Populating config"

echo '
[ChainStore]
  EnableSplitstore = true
[API]
  ListenAddress = "/ip4/0.0.0.0/tcp/1234/http"
' > $LOTUS_PATH/config.toml

echo "[*] Generate genesis for subnet deterministically"
if [[ "$IPC_SUBNET_ID" == "/r31415926" ]]; then
    eudico genesis new --subnet-id=$IPC_SUBNET_ID --template=/genesis-test.json --out=subnet.car
else
    eudico genesis new --subnet-id=$IPC_SUBNET_ID --template=/genesis.json --out=subnet.car
fi
echo "[*] Starting daemon"
eudico mir daemon --genesis=subnet.car --bootstrap=false
