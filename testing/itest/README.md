# Child Subnet Infra 
This is a utility script to spawn child subnet infrastructure, mainly for integration testing. 

It spawns the required number of eudico nodes and validators based on input config parameters. This would be a time saver 
when testing subnet checkpoints and cross net messages.

The `src/infra` contains the process flow for spawn subnet nodes and their validators.
The `examples` folder contains all the sample scripts to start different configurations and requirements.

## Requirements
In order to run these scripts, one must first deploy an IPC rootnet locally, as well as an IPC agent already configured with that network. These integration tests needs access to a `eudico` compiled binary, and assuments that the rootnet is deployed in the local environment, without any kind of virtualization or remote connection.

## Usage
Once a root net and the IPC agent have been manually configured and spawned. We can compile the itests through:
```shell
cargo build --examples --release
```

We'll then need to configure a set of environmental variables by exporting them manually, or configuring an `.env` file similar to the one shared in `env.template` and `source .env` in the terminal where the tests will be run.
```shell
# start the root subnet and ipc agent on your own

# define the following env variable
# the path to eudico binary path
export EUDICO_BIN=
# the ipc root folder that contains all the configs, i.e. ~/.ipc-agent
export IPC_ROOT_FOLDER=
# the parent subnet lotus path
export PARENT_LOTUS_PATH=
# the parent subnet id
export PARENT_SUBNET_ID=
# the name of the subnet
export SUBNET_NAME=
```

We can then run any of the infra scripts available in the `examples` folder. This will deploy the subnet-specific infra for the subnets:
```shell
# start the infra that runs util kill signal
../../target/release/examples/no_tear_down
```

Finally, we can run the actual tests by exporting the specific environmental variables required for the test in the corresponding terminal (if not done in previous steps):
```shell
# in another terminal, define the following variables
# the address that performs the funding and release across subnets 
export FUND_ADDRESS=
# the ipc agent json rpc url 
export IPC_AGENT_JSON_RPC_URL=http://localhost:3030/json_rpc
# child subnet id in string format
export CHILD_SUBNET_ID_STR=
```
And running the tests: 
```shell
# start the tests with 
cargo test -p itest --test checkpoint -- --nocapture
```

