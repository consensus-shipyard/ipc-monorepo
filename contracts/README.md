# IPC Solidity Actors

[![SAST](https://github.com/consensus-shipyard/ipc-solidity-actors/actions/workflows/sast.yaml/badge.svg)](https://github.com/consensus-shipyard/ipc-solidity-actors/actions/workflows/sast.yaml)
[![Tests](https://github.com/consensus-shipyard/ipc-solidity-actors/actions/workflows/test.yml/badge.svg)](https://github.com/consensus-shipyard/ipc-solidity-actors/actions/workflows/test.yml)

**‼️ The IPC Agent, the IPC actors (Rust, Solidity), and Eudico haven't been audited, tested in depth, or otherwise verified.
Moreover, the system is missing critical recovery functionality in case of crashes.
There are multiple ways in which you may lose funds moved into an IPC subnet,
and we strongly advise against deploying IPC on mainnet and/or using it with tokens with real value.**

This repository includes the reference implementation of all the actors (i.e. smart contracts)
responsible for the operation of the IPC (Inter-Planetary Consensus) protocol.
These actors are written in Solidity and target Filecoin’s FEVM.

The project accommodates the following main contracts

-   `GatewayDiamond.sol`: Implementation of the IPC Gateway within the Diamond pattern.
-   `SubnetActorDiamond.sol`: Reference implementation of an IPC SubnetActor within the Diamond pattern.
-   `SubnetRegistry.sol`: Registry contract for seamlessly deploying subnet actors.

# Documentation

## High-level Overview

The original idea of IPC is presented in these [paper](https://research.protocol.ai/publications/hierarchical-consensus-a-horizontal-scaling-framework-for-blockchains/delarocha2022.pdf), [post](https://docs.filecoin.io/basics/interplanetary-consensus/overview/) and [video](https://www.youtube.com/watch?v=G7d5KNRZdp0). The protocol has evolved a lot since the original paper, so take it as a high-level description of the system.

## Specification

The current specification draft is available [here](https://github.com/consensus-shipyard/IPC-design-reference-spec/blob/main/main.pdf).

# Deploying IPC Solidity contracts

Before deploying the contract, you'll need to configure the `RPC_URL` and `PRIVATE_KEY` environmental variables
to point to your network provider and the private key of the address you want to use for the deployment, respectively.

Alternatively, you can rename the `env.template` file included in the repo to `.env`, set your variables there,
and run `source .env` before running the deployment scripts.

To deploy the IPC Solidity contracts in an FEVM network, you can directly run the following:

```bash
make deploy-ipc
```

The scripts run by `make` make use of hardhat under the hood. If no network has been configured, the script will automatically try to fetch the chainID of the target network, and perform the deployment according to the configuration in `hardhat.config.ts`.
To deploy the contracts in some other network configured in the Hardhat config you can run the following:

```bash
make deploy-ipc NETWORK=<network-name>
```

# Upgrading IPC Solidity Contracts

This repository's contracts use the Diamond pattern for upgradability, allowing new features to be added or issues to be corrected without a full redeployment. The upgrade process is automated and includes bytecode verification to ensure the integrity of the changes.

## Automated Upgrade and Bytecode Verification

When you run an upgrade command, the repository's scripts handle several tasks:

1. **Bytecode Verification**: The scripts fetch the bytecode of the currently deployed contracts on an FEVM-powered IPC network using the details stored in local JSON files in the root directory of the git repository. They compare this with the bytecode generated after applying the intended changes on a temporary Ganache network.

2. **Conditional Upgrades**: If the bytecode verification process detects changes that align with the intended upgrades, the `make` command conditionally triggers other scripts to perform the actual upgrade on the network.

## Upgrade Commands

To upgrade a contract, you may use the following commands. The NETWORK parameter is optional; if not specified, the scripts will default to "auto":

-   **Gateway Diamond Upgrade**:

    ```bash
    make upgrade-gw-diamond [NETWORK=<network-name>]
    ```

-   **Subnet Actor Diamond Upgrade**:

    ```bash
    make upgrade-sa-diamond [NETWORK=<network-name>]
    ```

-   **Subnet Registry Diamond Upgrade**:
    ```bash
    make upgrade-sr-diamond [NETWORK=<network-name>]
    ```

After running any of these commands, the scripts will provide transaction details for verification. Check the transaction on the appropriate block explorer to confirm the upgrade's success.

## Important Notes

-   The upgrade commands are intended for use by authorized personnel with a deep understanding of the contracts' functionality.
-   Ensure that your local repository is up to date with the latest contract code and JSON files before initiating an upgrade.
-   Backup all contract data and thoroughly test any new code in a controlled environment prior to an upgrade.
-   Monitor the output of the upgrade process carefully for transaction details and to verify its successful completion.

## Branching Strategy

### Production branch

The production branch is `main`.
The `main` branch is always compatible with the "stable" release of the IPC agent that's running on Spacenet.
Updates to `main` **always** come from the `dev` branch.

### Development branch

The primary development branch is `dev`.
`dev` contains the most up-to-date software but may not be compatible with the version of the contracts deployed on spacenet and the `main` branch of the IPC agent. Only use `dev` if doing a full local deployment, but note that the packaged deployment scripts default to checking out `main` branches instead of `dev`.

# Actors overview

## Gateway

The purpose of the Gateway is to

1. serve as a register for the subnets of a given chain, dictating the rules for adding and removing new subnets
2. route cross chain messages
    1. store messages that are traveling from upper subnets in the hierarchy down to subnets that are on the same branch of their own chain (top-down messages)
    2. prepare epoch-defined checkpoints that collect messages traveling from lower levels of the hierarchy upwards (bottom-up messages)
3. distribute rewards to SubnetActors of child subnets

## SubnetActor

The purpose of the SubnetActor is to

1. keep track of a subnet’s parameters (name, parent, consensus, staking parameters, status, etc.)
2. provide validators with the ability to join and leave the subnet
3. manage validators’ stake
4. manage the subnet’s status
5. allows validators to submit checkpoints and to commit them to the Gateway once the majority is reached
6. distribute rewards, received from the Gateway, to validators

## SubnetRegistry

The purpose of the SubnetRegistry is to

1. deploy instances of the SubnetActor. Its role is to be the subnet actor factory in a subnet.

# Building & Testing with Forge

To build all contracts, run

```bash
forge build
```

The build artifacts (contracts’ ABI `.json` files), can be found in the `out` directory.

To run all repo tests run

```bash
forge test -vvv --ffi
```

And to generate coverage report run

```bash
forge coverage
```

To create the Rust bindings for the contract you can run:

```bash
make compile-abi && make rust-binding
```

# Development

Run `make install-dev` to install all necessary dependencies for development.

Before committing:

```bash
make fmt
make lint
make test
make slither
```

or

```bash
make prepare
```

Use `make storage` to check that the storage layout has not been corrupted.

Use `make coverage` to get the test coverage report.

# Bugs

Please report any bugs using the [issue tracker](https://github.com/consensus-shipyard/ipc-solidity-actors/issues).
