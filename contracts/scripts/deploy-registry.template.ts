import { deployContractWithDeployer, getTransactionFees } from './util'
import { ethers } from 'hardhat'

const { getSelectors, FacetCutAction } = require('./js/diamond.js')

export async function deploy() {
    const [deployer] = await ethers.getSigners()
    const balance = await deployer.getBalance()

    console.log(
        `Deploying contracts with account: ${
            deployer.address
        } and balance: ${balance.toString()}`,
    )

    const gatewayAddress = GATEWAY.Gateway
    const txArgs = await getTransactionFees()

    // deploy
    const getterFacet = await deployContractWithDeployer(
        deployer,
        'SubnetActorGetterFacet',
        {
            SubnetIDHelper: LIBMAP['SubnetIDHelper'],
        },
        txArgs,
    )
    const getterSelectors = getSelectors(getterFacet)
    // console.log("getter address:", getterFacet.address);

    const managerFacet = await deployContractWithDeployer(
        deployer,
        'SubnetActorManagerFacet',
        {},
        txArgs,
    )
    const managerSelectors = getSelectors(managerFacet)
    // console.log("manager address:", managerFacet.address);

    const pauserFacet = await deployContractWithDeployer(
        deployer,
        'SubnetActorPauseFacet',
        {},
        txArgs,
    )
    const pauserSelectors = getSelectors(pauserFacet)

    const rewarderFacet = await deployContractWithDeployer(
        deployer,
        'SubnetActorRewardFacet',
        { LibStaking: LIBMAP['LibStaking'] },
        txArgs,
    )
    const rewarderSelectors = getSelectors(rewarderFacet)

    const checkpointerFacet = await deployContractWithDeployer(
        deployer,
        'SubnetActorCheckpointingFacet',
        {},
        txArgs,
    )
    const checkpointerSelectors = getSelectors(checkpointerFacet)

    //deploy subnet registry diamond
    const registry = await ethers.getContractFactory('SubnetRegistryDiamond', {
        signer: deployer,
    })

    const registryConstructorParams = {
        gateway: gatewayAddress,
        getterFacet: getterFacet.address,
        managerFacet: managerFacet.address,
        rewarderFacet: rewarderFacet.address,
        pauserFacet: pauserFacet.address,
        checkpointerFacet: checkpointerFacet.address,
        subnetGetterSelectors: getterSelectors,
        subnetManagerSelectors: managerSelectors,
        checkpointerSelectors: checkpointerSelectors,
        pauserSelectors: pauserSelectors,
        rewarderSelectors: rewarderSelectors,
    }

    const facetCuts = [] //TODO

    const facets = [
        {
            name: 'RegisterSubnetFacet',
            libs: {
                SubnetIDHelper: LIBMAP['SubnetIDHelper'],
            },
        },
        { name: 'SubnetGetterFacet', libs: {} },
        { name: 'DiamondLoupeFacet', libs: {} },
        { name: 'DiamondCutFacet', libs: {} },
    ]

    for (const facet of facets) {
        const facetInstance = await deployContractWithDeployer(
            deployer,
            facet.name,
            facet.libs,
            txArgs,
        )
        await facetInstance.deployed()

        facet.address = facetInstance.address

        facetCuts.push({
            facetAddress: facetInstance.address,
            action: FacetCutAction.Add,
            functionSelectors: getSelectors(facetInstance),
        })
    }

    const diamondLibs = {
        SubnetIDHelper: LIBMAP['SubnetIDHelper'],
    }
    // deploy Diamond
    const { address: subnetRegistryAddress } = await deployContractWithDeployer(
        deployer,
        'SubnetRegistryDiamond',
        {},
        facetCuts,
        registryConstructorParams,
        txArgs,
    )

    return {
        SubnetRegistry: subnetRegistryAddress,
        Facets: facets,
    }
}
