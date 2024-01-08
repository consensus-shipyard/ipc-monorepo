# IPC Quick Start: zero-to-subnet (on Calibration)

>💡 Background and details are available in the [README](/README.md).

Ready to test the waters with your first subnet? This guide will deploy a subnet with three local validators orchestrated by `ipc-cli`. This subnet will be anchored to the public [Calibration testnet](https://docs.filecoin.io/networks/calibration/details/). This will be a minimal example and may not work on all systems. The full documentation provides more details on each step.

Several steps in this guide involve running long-lived processes. In each of these cases, the guide advises starting a new *session*. Depending on your set-up, you may do this using tools like `screen` or `tmux`, or, if using a graphical environment, by opening a new terminal tab, pane, or window.

<!-- >💡A video walkthrough of this guide is currently being prepared. We still encourage you to try it for yourself! -->

<!-- >💡If you're only looking to connect to an existing subnet, please see the [README](deploying-hierarchy.md) instead. -->

## Step 0: Prepare your system

We assume an Ubuntu Linux instance when discussing prerequisites, but annotate steps with system-specificity and links to detailed multi-OS instructions. Exact procedures will vary for other systems, so please follow the links if running something different. Details on IPC-specific requirements can also be found in the [README](/README.md).

* Install basic dependencies [Ubuntu/Debian] ([details](https://lotus.filecoin.io/lotus/install/prerequisites/#supported-platforms))
```bash
sudo apt update && sudo apt install build-essential libssl-dev mesa-opencl-icd ocl-icd-opencl-dev gcc git bzr jq pkg-config curl clang hwloc libhwloc-dev wget ca-certificates gnupg -y 
```

* Install Rust [Linux] ([details](https://www.rust-lang.org/tools/install))
```bash
curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"
```

* Install Docker Engine [Ubuntu] ([details](https://docs.docker.com/engine/install/))
```bash
sudo install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
sudo chmod a+r /etc/apt/keyrings/docker.gpg
echo \
  "deb [arch="$(dpkg --print-architecture)" signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu \
  "$(. /etc/os-release && echo "$VERSION_CODENAME")" stable" | \
  sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
sudo apt-get update && sudo apt-get install docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin -y
sudo usermod -aG docker $USER && newgrp docker
```


## Step 1: Build the IPC stack

Next, we'll download and build the different components (mainly, `ipc-cli` and Fendermint).

* Pick a folder where to build the IPC stack.
* Download and compile the `ipc-cli`.
```bash
git clone https://github.com/consensus-shipyard/ipc.git
(cd ipc && make build && make install-infra)
```

## Step 2: Initialise your config

* Initialise the config
```bash
./bin/ipc-cli config init
```

This should have populated a default config file with all the parameters required to connect to calibration at `~/.ipc/config.toml`. Feel free to update this configuration to fit your needs. You may need to replace the content of the config to reflect the address of the up-to-date contracts in Calibration.


* You can run `nano ~/.ipc/config.toml` to double-check that the config file has been populated with the following content:
```toml
keystore_path = "~/.ipc"

[[subnets]]
id = "/r314159"

[subnets.config]
network_type = "fevm"
provider_http = "https://api.calibration.node.glif.io/rpc/v1"
gateway_addr = "0x0341fA160C66aBB112195192aE359a6D61df45cd"
registry_addr = "0xc7068Cea947035560128a6a6F4c8913523A5A44C"

# Subnet template - uncomment and adjust before using
# [[subnets]]
# id = "/r314159/<SUBNET_ID>"

# [subnets.config]
# network_type = "fevm"
# provider_http = "<RPC_ADDR>"
# gateway_addr = "0x77aa40b105843728088c0132e43fc44348881da8"
# registry_addr = "0x74539671a1d2f1c8f200826baba665179f53a1b7"
```

## Step 3: Set up your wallets

You'll need to create a set of wallets to spawn and interact of the subnet. Please make a note of the addresses as you go along, it may make your life easier.

* Create the three different wallets
```bash
./bin/ipc-cli wallet new -w evm
./bin/ipc-cli wallet new -w evm
./bin/ipc-cli wallet new -w evm
```

* You can optionally set one of the wallets as your default so you don't have to use the `--from` flag explicitly in some of the commands:
```bash
./bin/ipc-cli wallet set-default --address <DEFAULT_ETH_ADDR> -w evm
```

<!-- * Convert the 0x addresses to f4 addresses for later usage (OWNER_1_F4, OWNER_2_F4, and OWNER_3_F4)  -->
<!-- ```bash -->
<!-- ./ipc-agent/bin/ipc-agent util eth-to-f4-addr --addr <OWNER_1> -->
<!-- ./ipc-agent/bin/ipc-agent util eth-to-f4-addr --addr <OWNER_2> -->
<!-- ./ipc-agent/bin/ipc-agent util eth-to-f4-addr --addr <OWNER_3> -->
<!-- ``` -->

* Go to the [Calibration faucet](https://faucet.calibration.fildev.network/) and get some funds sent to each of your addresses 

>💡 In case you'd like to import an EVM account into Metamask, you can use export the private key using `./bin/ipc-cli wallet export -w evm -a <ADDRESS>`. More information is available in the [EVM IPC agent support docs](./usage.md#key-management).

>💡 Note that you may hit faucet rate limits. In that case, wait a few minutes or continue with the guide and come back to this before step 9. Alternatively, you can send funds from your primary wallet to your owner wallets.


## Step 4: Create a child subnet

* The next step is to create a subnet under `/r314159` in calibration. Remember to set a default wallet or explicitly specifying the wallet from which you want to perform the action with the `--from` flag.
```bash
./bin/ipc-cli subnet create --parent /r314159 --min-validators 3 --min-validator-stake 1 --bottomup-check-period 30
```

* Make a note of the address of the subnet you created.

## Step 5: Join the subnet

Before we deploy the infrastructure for the subnet, we will have to bootstrap the subnet and join from our validators, putting some initial collateral into the subnet and giving our validator address some initial balance in the subnet. For this, we need to send a `join` command from each of our validators from their validator owner addresses providing their corresponding public key.

* Get the public key for all of your wallets and note it down. This is the public key that each of your validators will use to sign blocks in the subnet.
```bash
./bin/ipc-cli wallet pub-key -w evm --address <WALLET_ADDR1>
./bin/ipc-cli wallet pub-key -w evm --address <WALLET_ADDR2>
./bin/ipc-cli wallet pub-key -w evm --address <WALLET_ADDR3>
```

* Join the subnet with each validator
```bash
./bin/ipc-cli subnet join --from=<WALLET_ADDR1> --subnet=/r314159/<SUBNET_ID> --collateral=10 --public-key=<PUBKEY_WALLET1> --initial-balance 1
./bin/ipc-cli subnet join --from=<WALLET_ADDR2> --subnet=/r314159/<SUBNET_ID> --collateral=10 --public-key=<PUBKEY_WALLET2> --initial-balance 1
./bin/ipc-cli subnet join --from=<WALLET_ADDR3> --subnet=/r314159/<SUBNET_ID> --collateral=10 --public-key=<PUBKEY_WALLET3> --initial-balance 1
```

## Step 6: Deploy the infrastructure
With the collateral and number of minimum validators fulfilled, the subnet is bootstrapped in the parent, and we can deploy the infrastructure.

### Deploying a bootstrap node
Before running our validators, at least one bootstrap needs to be deployed and advertised in the network. Bootstrap nodes allow validators discover other peers and validators in the network. In the current implementation of IPC, only validators are allowed to advertise bootstrap nodes.

* We can deploy a new bootstrap node in the subnet by running: 
```bash
cargo make --makefile infra/Makefile.toml \
    -e PRIVATE_KEY_PATH=<VALIDATOR_PRIV_KEY> \
    -e SUBNET_ID=<SUBNET_ID> \
    -e CMT_P2P_HOST_PORT=<COMETBFT_P2P_PORT> \
    -e CMT_RPC_HOST_PORT=<COMETBFT_RPC_PORT> \
    -e ETHAPI_HOST_PORT=<ETH_RPC_PORT> \
    -e BOOTSTRAPS=<BOOTSTRAP_ENDPOINT>
    -e PARENT_REGISTRY=<PARENT_REGISTRY_CONTRACT_ADDR> \
    -e PARENT_GATEWAY=<GATEWAY_REGISTRY_CONTRACT_ADDR> \
    -e CMT_EXTERNAL_ADDR=<COMETBFT_EXTERNAL_ENDPOINT> \
    bootstrap
```

At the end of the output, this command should return the ID of your new bootstrap node:
```console

[cargo-make] INFO - Running Task: cometbft-wait
[cargo-make] INFO - Running Task: cometbft-node-id
2b23b8298dff7711819172252f9df3c84531b1d9@172.26.0.2:26650
[cargo-make] INFO - Build Done in 13.38 seconds.
```
Remember the address of your bootstrap for the next step. This address has the following format `id@ip:port`. Feel free to adjust the `ip` to use a reachable IP for your deployment so other nodes can contact it (in our case our localhost IP, `127.0.0.1`) and to use as `CMT_EXTERNAL_ADDR` a reachable IP by other peers, and the port configured in `CMT_EXTERNAL_ADDR`.

* We can get the address of the deployed bootstrap node by running:
```bash
cargo make --makefile bin/ipc-infra/Makefile.toml bootstrap-node-id
```

* To shut down the bootstrap node run:
```bash
cargo make --makefile bin/ipc-infra/Makefile.toml bootstrap-down
```

* To advertise the endpoint to the rest of nodes in the network we need to run:
```bash
# Example of BOOTSTRAP_ENDPOINT = 2b23b8298dff7711819172252f9df3c84531b1d9@172.26.0.2:26650
./bin/ipc-cli subnet add-bootstrap --subnet=<SUBNET_ID> --endpoint="<BOOTSRAP_ENDPOINT>"
```

* The bootstrap nodes currently deployed in the network can be queried through the following command:
```bash
./bin/ipc-cli subnet list-bootstraps --subnet=<SUBNET_ID>
```

### Deploying the validator infrastructure
With the bootstrap node deployed and advertised to the network, we are now ready to deploy the validators that will run the subnet.

* First we need to export the private keys of our validators from the addresses that we created with our `ipc-cli wallet` to a known path so they can be picked by Fendermint to sign blocks. We can use the default repo of IPC for this, `~/.ipc`.
```bash
./bin/ipc-cli wallet export -w evm -a <WALLET_ADDR1> --hex -o ~/.ipc/<PRIV_KEY_VALIDATOR_1>
./bin/ipc-cli wallet export -w evm -a <WALLET_ADDR2> --hex -o ~/.ipc/<PRIV_KEY_VALIDATOR_2>
./bin/ipc-cli wallet export -w evm -a <WALLET_ADDR3> --hex -o ~/.ipc/<PRIV_KEY_VALIDATOR_3>
```

* Now we have all that we need to deploy the three validators using the following command (configured for each of the validators, i.e. replace the arguments with `<..-n>` to fit that of the specific validator).

```bash
cargo make --makefile ./bin/ipc-infra/Makefile.toml \
    -e NODE_NAME=validator-<n> \
    -e PRIVATE_KEY_PATH=<PATH_PRIV_KEY_VALIDATOR_n> \
    -e SUBNET_ID=<SUBNET_ID> \
    -e CMT_P2P_HOST_PORT=<COMETBFT_P2P_PORT_n> -e CMT_RPC_HOST_PORT=<COMETBFT_RPC_PORT_n> \
    -e ETHAPI_HOST_PORT=<ETH_RPC_PORT_n> \
    -e BOOTSTRAPS=<BOOTSTRAP_ENDPOINT> \
    -e PARENT_REGISTRY=<PARENT_REGISTRY_CONTRACT_ADDR> \
    -e PARENT_GATEWAY=<GATEWAY_REGISTRY_CONTRACT_ADDR> \
    -e CMT_EXTERNAL_ADDR=<COMETBFT_EXTERNAL_ENDPOINT> \
    child-validator
```
`PARENT_REGISTRY` and `PARENT_GATEWAY` are the contract addresses of the IPC contracts in Calibration. This command also uses the calibration endpoint as default. Finally, you'll need to choose a different `NODE_NAME`, `CMT_HOST_PORT`, `ETHAPI_HOST_PORT` for each of the validators.

With this, we have everything in place, and our subnet should start automatically validating new blocks. You can find additional documentation on how to run the infrastructure in the [Fendermint docs](https://github.com/consensus-shipyard/fendermint/blob/main/docs/ipc.md).

## Step 7: Configure your subnet in the IPC CLI

* Edit the `ipc-cli` configuration `config.toml`
```bash
nano ~/.ipc/config.toml
```

* Append the new subnet to the configuration
```toml
[[subnets]]
id = "/r314159"

[subnets.config]
network_type = "fevm"
provider_http = "http://127.0.0.1:<ETH_RPC_PORT>"
gateway_addr = "0x77aa40b105843728088c0132e43fc44348881da8"
registry_addr = "0x74539671a1d2f1c8f200826baba665179f53a1b7"
```

With this you should be able to start interacting with your local subnet directly through your `ipc-cli`. You can try to fetch the balances of your wallets through the following command. The result should show the initial balance that you have included for your validators address in genesis:
```bash
./bin/ipc-cli wallet balances -w evm --subnet=<SUBNET_ID>
```

> The ETH addresses for `gateway_addr` and `registry_addr` used when they are deployed in genesis in a child subnet by Fendermint are `0x77aa40b105843728088c0132e43fc44348881da8` and `0x74539671a1d2f1c8f200826baba665179f53a1b7`, respectively.

## Step 8: Interact with your the ETH RPC

For information about how to connect your Ethereum tooling with your subnet refer to the [following docs](./contracts.md).

## Step 9 (optional): Run a relayer
IPC relies on the role of a specific type of peer on the network called the relayers that are responsible for submitting bottom-up checkpoints that have been finalized in a child subnet to its parent. This process is key for the commitment of child subnet checkpoints in the parent, and the execution of bottom-up cross-net messages. Without relayers, cross-net messages will only flow from top levels of the hierarchy to the bottom, but not the other way around.

* *session* Run the relayer process for your subnet using your default address by calling:
```bash
./bin/ipc-cli checkpoint relayer --subnet <SUBNET_ID>
```
* In order to run the relayer from a different address you can use the `--submitted` flag: 
```bash
./bin/ipc-cli checkpoint relayer --subnet <SUBNET_ID> --submitter <RELAYER_ADDR>
```

Relayers are rewarded through cross-net messages fees for the timely submission of bottom-up checkpoints to the parent. In order to claim the checkpointing rewards collected for a subnet, the following command need to be run from the relayer address:
```bash
./bin/ipc-cli subnet claim --subnet=<SUBNET_ID> --reward
```

## Step 10: What now?
* Proceed to the [usage](usage.md) guide to learn how you can test your new subnet.
* If something went wrong, please have a look at the [README](https://github.com/consensus-shipyard/ipc). If it doesn't help, please join us in #ipc-help. In either case, let us know your experience!
* Please note that to repeat this guide or spawn a new subnet, you may need to change the parameters or reset your system.
