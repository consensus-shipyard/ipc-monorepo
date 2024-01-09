import '@nomicfoundation/hardhat-foundry'
import '@nomiclabs/hardhat-ethers'
import '@typechain/hardhat'
import dotenv from 'dotenv'
import fs from 'fs'
import 'hardhat-contract-sizer'
import 'hardhat-deploy'
import 'hardhat-storage-layout-changes'
import { HardhatUserConfig, task } from 'hardhat/config'
import { HardhatRuntimeEnvironment } from 'hardhat/types'

dotenv.config()

const lazyImport = async (module: any) => {
    return await import(module)
}

async function saveDeployments(
    env: string,
    deploymentData: { [key in string]: string },
    branch?: string,
) {
    const deploymentsJsonPath = `${process.cwd()}/deployments.json`

    let deploymentsJson = { [env]: {} }
    if (fs.existsSync(deploymentsJsonPath)) {
        deploymentsJson = JSON.parse(
            fs.readFileSync(deploymentsJsonPath).toString(),
        )
    }

    if (branch) {
        deploymentsJson[env] = {
            ...deploymentsJson[env],
            [branch]: deploymentData,
        }
    } else {
        deploymentsJson[env] = { ...deploymentsJson[env], ...deploymentData }
    }

    fs.writeFileSync(deploymentsJsonPath, JSON.stringify(deploymentsJson))
}

async function saveDeploymentsFacets(
    filename: string,
    env: string,
    updatedFacets: { [key: string]: string },
    branch?: string,
) {
    const deploymentsJsonPath = `${process.cwd()}/${filename}`
    let deploymentsJson = { [env]: {} }
    if (fs.existsSync(deploymentsJsonPath)) {
        deploymentsJson = JSON.parse(
            fs.readFileSync(deploymentsJsonPath).toString(),
        )
    }
    const facets = deploymentsJson[env]['Facets']
    for (const facetIndex in facets) {
        const facetName = facets[facetIndex].name
        if (updatedFacets[facetName]) {
            facets[facetIndex].address = updatedFacets[facetName]
        }
    }
    fs.writeFileSync(deploymentsJsonPath, JSON.stringify(deploymentsJson))
}
async function saveSubnetRegistry(
    env: string,
    subnetRegistryData: { [key in string]: string },
) {
    const subnetRegistryJsonPath = `${process.cwd()}/subnet.registry.json`

    let subnetRegistryJson = { [env]: {} }
    if (fs.existsSync(subnetRegistryJsonPath)) {
        subnetRegistryJson = JSON.parse(
            fs.readFileSync(subnetRegistryJsonPath).toString(),
        )
    }

    subnetRegistryJson[env] = {
        ...subnetRegistryJson[env],
        ...subnetRegistryData,
    }

    fs.writeFileSync(subnetRegistryJsonPath, JSON.stringify(subnetRegistryJson))
}

async function saveSubnetActor(
    env: string,
    subnetRegistryData: { [key in string]: string },
) {
    const subnetRegistryJsonPath = `${process.cwd()}/subnet.actor.json`

    let subnetRegistryJson = { [env]: {} }
    if (fs.existsSync(subnetRegistryJsonPath)) {
        subnetRegistryJson = JSON.parse(
            fs.readFileSync(subnetRegistryJsonPath).toString(),
        )
    }

    subnetRegistryJson[env] = {
        ...subnetRegistryJson[env],
        ...subnetRegistryData,
    }

    fs.writeFileSync(subnetRegistryJsonPath, JSON.stringify(subnetRegistryJson))
}

async function getSubnetRegistry(
    env: string,
): Promise<{ [key in string]: string }> {
    const subnetRegistryJsonPath = `${process.cwd()}/subnet.registry.json`

    let subnetRegistry = {}
    if (fs.existsSync(subnetRegistryJsonPath)) {
        subnetRegistry = JSON.parse(
            fs.readFileSync(subnetRegistryJsonPath).toString(),
        )[env]
    }

    return subnetRegistry
}

async function getSubnetActor(
    env: string,
): Promise<{ [key in string]: string }> {
    const subnetRegistryJsonPath = `${process.cwd()}/subnet.actor.json`

    let subnetRegistry = {}
    if (fs.existsSync(subnetRegistryJsonPath)) {
        subnetRegistry = JSON.parse(
            fs.readFileSync(subnetRegistryJsonPath).toString(),
        )[env]
    }

    return subnetRegistry
}

async function getDeployments(
    env: string,
): Promise<{ [key in string]: string }> {
    const deploymentsJsonPath = `${process.cwd()}/deployments.json`

    let deployments = {}
    if (fs.existsSync(deploymentsJsonPath)) {
        deployments = JSON.parse(
            fs.readFileSync(deploymentsJsonPath).toString(),
        )[env]
    }

    return deployments
}

task(
    'deploy-libraries',
    'Build and deploys all libraries on the selected network',
    async (args, hre: HardhatRuntimeEnvironment) => {
        const { deploy } = await lazyImport('./scripts/deploy-libraries')
        const libsDeployment = await deploy()
        console.log(libsDeployment)
        await saveDeployments(hre.network.name, libsDeployment, 'libs')
    },
)

task(
    'deploy-gateway',
    'Builds and deploys the Gateway contract on the selected network',
    async (args, hre: HardhatRuntimeEnvironment) => {
        const network = hre.network.name

        const deployments = await getDeployments(network)
        const { deploy } = await lazyImport('./scripts/deploy-gateway')
        const gatewayDeployment = await deploy(deployments.libs)

        console.log(JSON.stringify(gatewayDeployment, null, 2))

        await saveDeployments(network, gatewayDeployment)
    },
)

task(
    'deploy-subnet-registry',
    'Builds and deploys the Registry contract on the selected network',
    async (args, hre: HardhatRuntimeEnvironment) => {
        const network = hre.network.name
        const { deploy } = await lazyImport('./scripts/deploy-registry')
        const subnetRegistryDeployment = await deploy()

        console.log(JSON.stringify(subnetRegistryDeployment, null, 2))

        await saveSubnetRegistry(network, subnetRegistryDeployment)
    },
)

task(
    'deploy-gw-diamond-and-facets',
    'Builds and deploys Gateway Actor diamond and its facets',
    async (args, hre: HardhatRuntimeEnvironment) => {
        const network = hre.network.name
        const deployments = await getDeployments(network)
        const { deployDiamond } = await lazyImport(
            './scripts/deploy-gw-diamond',
        )
        const gatewayActorDiamond = await deployDiamond(deployments.libs)
        await saveDeployments(network, gatewayActorDiamond)
    },
)

task(
    'deploy-sa-diamond-and-facets',
    'Builds and deploys Subnet Actor diamond and its facets',
    async (args, hre: HardhatRuntimeEnvironment) => {
        await hre.run('compile')

        const network = hre.network.name
        const deployments = await getDeployments(network)
        const { deployDiamond } = await lazyImport(
            './scripts/deploy-sa-diamond',
        )
        const subnetActorDiamond = await deployDiamond(
            deployments.Gateway,
            deployments.libs,
        )
        await saveSubnetActor(network, subnetActorDiamond)
    },
)

task(
    'deploy',
    'Builds and deploys all contracts on the selected network',
    async (args, hre: HardhatRuntimeEnvironment) => {
        await hre.run('compile')
        await hre.run('deploy-libraries')
        await hre.run('deploy-gateway')
    },
)

task(
    'deploy-gw-diamond',
    'Builds and deploys Gateway Actor diamond',
    async (args, hre: HardhatRuntimeEnvironment) => {
        await hre.run('compile')
        await hre.run('deploy-libraries')
        await hre.run('deploy-gw-diamond-and-facets')
    },
)

task(
    'deploy-sa-diamond',
    'Builds and deploys Subnet Actor diamond',
    async (args, hre: HardhatRuntimeEnvironment) => {
        await hre.run('compile')
        await hre.run('deploy-libraries')
        await hre.run('deploy-sa-diamond-and-facets')
    },
)

task(
    'upgrade-gw-diamond',
    'Upgrades IPC Gateway Actor Diamond Facets on an EVM-compatible subnet using hardhat',
    async (args, hre: HardhatRuntimeEnvironment) => {
        await hre.run('compile')
        const network = hre.network.name
        const deployments = await getDeployments(network)
        const { upgradeDiamond } = await lazyImport(
            './scripts/upgrade-gw-diamond',
        )
        const updatedFacets = await upgradeDiamond(deployments)
        await saveDeploymentsFacets('deployments.json', network, updatedFacets)
    },
)

task(
    'upgrade-sr-diamond',
    'Upgrades IPC Subnet Registry Diamond Facets on an EVM-compatible subnet using hardhat',
    async (args, hre: HardhatRuntimeEnvironment) => {
        await hre.run('compile')
        const network = hre.network.name
        const subnetRegistry = await getSubnetRegistry(network)
        const { upgradeDiamond } = await lazyImport(
            './scripts/upgrade-sr-diamond',
        )
        const updatedFacets = await upgradeDiamond(subnetRegistry)
        await saveDeploymentsFacets(
            'subnet.registry.json',
            network,
            updatedFacets,
        )
    },
)

task(
    'upgrade-sa-diamond',
    'Upgrades IPC Subnet Actor Diamond Facets on an EVM-compatible subnet using hardhat',
    async (args, hre: HardhatRuntimeEnvironment) => {
        await hre.run('compile')
        const network = hre.network.name
        const deployments = await getSubnetActor(network)
        const { upgradeDiamond } = await lazyImport(
            './scripts/upgrade-sa-diamond',
        )
        const updatedFacets = await upgradeDiamond(deployments)
        await saveDeploymentsFacets('subnet.actor.json', network, updatedFacets)
    },
)

/** @type import('hardhat/config').HardhatUserConfig */
const config: HardhatUserConfig = {
    defaultNetwork: 'calibrationnet',
    networks: {
        mainnet: {
            chainId: 314,
            url: process.env.RPC_URL!,
            accounts: [process.env.PRIVATE_KEY!],
            timeout: 1000000,
        },
        calibrationnet: {
            chainId: 314159,
            url: process.env.RPC_URL!,
            accounts: [process.env.PRIVATE_KEY!],
            timeout: 1000000,
        },
        localnet: {
            chainId: 31415926,
            url: process.env.RPC_URL!,
            accounts: [process.env.PRIVATE_KEY!],
        },
        // automatically fetch chainID for network
        auto: {
            chainId: parseInt(process.env.CHAIN_ID!, 16),
            url: process.env.RPC_URL!,
            accounts: [process.env.PRIVATE_KEY!],
            // timeout to support also slow networks (like calibration/mainnet)
            timeout: 1000000,
        },
    },
    solidity: {
        compilers: [
            {
                version: '0.8.19',
                settings: {
                    viaIR: true,
                    optimizer: {
                        enabled: true,
                        runs: 200,
                    },
                },
            },
        ],
    },
    typechain: {
        outDir: 'typechain',
        target: 'ethers-v5',
    },
    paths: {
        storageLayouts: '.storage-layouts',
    },
    storageLayoutChanges: {
        contracts: [
            'GatewayDiamond',
            'SubnetActorDiamond',
            'GatewayActorModifiers',
            'SubnetActorModifiers',
        ],
        fullPath: false,
    },
}

export default config
