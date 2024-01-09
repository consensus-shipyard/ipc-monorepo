// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.19;

import "../../src/errors/IPCErrors.sol";
import "forge-std/Test.sol";

import {ConsensusType} from "../../src/enums/ConsensusType.sol";
import {TestUtils} from "../helpers/TestUtils.sol";
import {IERC165} from "../../src/interfaces/IERC165.sol";
import {IDiamond} from "../../src/interfaces/IDiamond.sol";
import {IDiamondCut} from "../../src/interfaces/IDiamondCut.sol";
import {IDiamondLoupe} from "../../src/interfaces/IDiamondLoupe.sol";
import {LibDiamond} from "../../src/lib/LibDiamond.sol";

import {SubnetActorGetterFacet} from "../../src/subnet/SubnetActorGetterFacet.sol";
import {SubnetActorManagerFacet} from "../../src/subnet/SubnetActorManagerFacet.sol";
import {SubnetActorPauseFacet} from "../../src/subnet/SubnetActorPauseFacet.sol";
import {SubnetActorCheckpointingFacet} from "../../src/subnet/SubnetActorCheckpointingFacet.sol";
import {SubnetActorRewardFacet} from "../../src/subnet/SubnetActorRewardFacet.sol";
import {SubnetActorDiamond} from "../../src/SubnetActorDiamond.sol";
import {SubnetID, PermissionMode} from "../../src/structs/Subnet.sol";
import {SubnetRegistryDiamond} from "../../src/SubnetRegistryDiamond.sol";

import {RegisterSubnetFacet} from "../../src/subnetregistry/RegisterSubnetFacet.sol";
import {SubnetGetterFacet} from "../../src/subnetregistry/SubnetGetterFacet.sol";
import {DiamondLoupeFacet} from "../../src/diamond/DiamondLoupeFacet.sol";
import {DiamondCutFacet} from "../../src/diamond/DiamondCutFacet.sol";
import {SupplySourceHelper} from "../../src/lib/SupplySourceHelper.sol";

import {IntegrationTestBase, TestRegistry} from "../IntegrationTestBase.sol";

contract SubnetRegistryTest is Test, TestRegistry, IntegrationTestBase {
    bytes4[] empty;

    function setUp() public virtual override {
        bytes4[] memory mockedSelectors = new bytes4[](1);
        mockedSelectors[0] = 0x6cb2ecee;

        bytes4[] memory mockedSelectors2 = new bytes4[](1);
        mockedSelectors2[0] = 0x133f74ea;

        SubnetRegistryDiamond.ConstructorParams memory params;
        params.gateway = DEFAULT_IPC_GATEWAY_ADDR;
        params.getterFacet = address(new SubnetActorGetterFacet());
        params.managerFacet = address(new SubnetActorManagerFacet());
        params.subnetGetterSelectors = mockedSelectors;
        params.subnetManagerSelectors = mockedSelectors2;

        registryDiamond = createSubnetRegistry(params);
        registryLouper = DiamondLoupeFacet(address(registryDiamond));
        registryCutter = DiamondCutFacet(address(registryDiamond));
        registrySubnetFacet = RegisterSubnetFacet(address(registryDiamond));
        registrySubnetGetterFacet = SubnetGetterFacet(address(registryDiamond));
    }

    function test_Registry_FacetFunctionSelectors() public view {
        IDiamondLoupe.Facet[] memory facets;
        uint256 facetsLength = facets.length;
        for (uint256 i = 0; i < facetsLength; ++i) {
            address facetAddress = facets[i].facetAddress;
            require(
                registryLouper.facetFunctionSelectors(facetAddress).length == facets[i].functionSelectors.length,
                "unexpected function selector length"
            );
        }
    }

    function test_Registry_Deployment_IERC165() public view {
        require(registryLouper.facets().length == 4, "unexpected length");
        require(registryLouper.facetAddresses().length == registryLouper.facets().length, "inconsistent diamond size");
        require(registryLouper.supportsInterface(type(IERC165).interfaceId) == true, "IERC165 not supported");
        require(registryLouper.supportsInterface(type(IDiamondCut).interfaceId) == true, "IDiamondCut not supported");
        require(
            registryLouper.supportsInterface(type(IDiamondLoupe).interfaceId) == true,
            "IDiamondLoupe not supported"
        );
    }

    function test_Registry_Deployment_ZeroGetterFacet() public {
        SubnetRegistryDiamond.ConstructorParams memory params;
        params.gateway = DEFAULT_IPC_GATEWAY_ADDR;
        params.getterFacet = address(0);
        params.managerFacet = address(1);
        params.subnetGetterSelectors = empty;
        params.subnetManagerSelectors = empty;

        IDiamond.FacetCut[] memory diamondCut = new IDiamond.FacetCut[](0);
        vm.expectRevert(FacetCannotBeZero.selector);
        new SubnetRegistryDiamond(diamondCut, params);
    }

    function test_Registry_Deployment_ZeroManagerFacet() public {
        SubnetRegistryDiamond.ConstructorParams memory params;
        params.gateway = DEFAULT_IPC_GATEWAY_ADDR;
        params.getterFacet = address(1);
        params.managerFacet = address(0);
        params.subnetGetterSelectors = empty;
        params.subnetManagerSelectors = empty;

        IDiamond.FacetCut[] memory diamondCut = new IDiamond.FacetCut[](0);
        vm.expectRevert(FacetCannotBeZero.selector);
        new SubnetRegistryDiamond(diamondCut, params);
    }

    function test_Registry_Deployment_ZeroGateway() public {
        SubnetRegistryDiamond.ConstructorParams memory params;
        params.gateway = address(0);
        params.getterFacet = address(1);
        params.managerFacet = address(1);
        params.subnetGetterSelectors = empty;
        params.subnetManagerSelectors = empty;

        IDiamond.FacetCut[] memory diamondCut = new IDiamond.FacetCut[](0);
        vm.expectRevert(GatewayCannotBeZero.selector);
        new SubnetRegistryDiamond(diamondCut, params);
    }

    function test_Registry_Deployment_DifferentGateway() public {
        SubnetActorDiamond.ConstructorParams memory params = defaultSubnetActorParamsWithGateway(address(1));
        params.permissionMode = PermissionMode.Collateral;

        vm.expectRevert(WrongGateway.selector);
        registrySubnetFacet.newSubnetActor(params);
    }

    function test_Registry_LatestSubnetDeploy_Revert() public {
        vm.startPrank(DEFAULT_SENDER);

        SubnetActorDiamond.ConstructorParams memory params = defaultSubnetActorParamsWithGateway(
            DEFAULT_IPC_GATEWAY_ADDR
        );
        params.permissionMode = PermissionMode.Collateral;

        registrySubnetFacet.newSubnetActor(params);
        vm.expectRevert(CannotFindSubnet.selector);
        registrySubnetGetterFacet.latestSubnetDeployed(address(0));
    }

    function test_Registry_GetSubnetDeployedByNonce_Revert() public {
        vm.startPrank(DEFAULT_SENDER);

        SubnetActorDiamond.ConstructorParams memory params = defaultSubnetActorParamsWithGateway(
            DEFAULT_IPC_GATEWAY_ADDR
        );
        params.permissionMode = PermissionMode.Collateral;

        registrySubnetFacet.newSubnetActor(params);
        vm.expectRevert(CannotFindSubnet.selector);
        registrySubnetGetterFacet.getSubnetDeployedByNonce(address(0), 1);
    }

    function test_Registry_Deployment_Works() public {
        vm.startPrank(DEFAULT_SENDER);

        SubnetActorDiamond.ConstructorParams memory params = defaultSubnetActorParamsWithGateway(
            DEFAULT_IPC_GATEWAY_ADDR
        );
        registrySubnetFacet.newSubnetActor(params);
        require(registrySubnetGetterFacet.latestSubnetDeployed(DEFAULT_SENDER) != address(0));
    }

    function test_deploySubnetActor_fuzz(
        uint256 _minCollateral,
        uint64 _minValidators,
        uint64 _bottomUpCheckPeriod,
        uint16 _activeValidatorsLimit,
        uint8 _majorityPercentage,
        uint256 _minCrossMsgFee,
        uint8 _pathSize,
        int8 _powerScale
    ) public {
        vm.assume(_minCollateral > 0);
        vm.assume(_bottomUpCheckPeriod > 0);
        vm.assume(_majorityPercentage >= 51 && _majorityPercentage <= 100);
        vm.assume(_powerScale <= 18);
        vm.assume(_pathSize >= 0 && _pathSize <= 5);

        address[] memory path = new address[](_pathSize);
        for (uint8 i; i < _pathSize; ++i) {
            path[i] = vm.addr(300 + i);
        }

        SubnetActorDiamond.ConstructorParams memory params = SubnetActorDiamond.ConstructorParams({
            parentId: SubnetID({root: ROOTNET_CHAINID, route: path}),
            ipcGatewayAddr: DEFAULT_IPC_GATEWAY_ADDR,
            consensus: ConsensusType.Fendermint,
            minActivationCollateral: _minCollateral,
            minValidators: _minValidators,
            bottomUpCheckPeriod: _bottomUpCheckPeriod,
            majorityPercentage: _majorityPercentage,
            activeValidatorsLimit: _activeValidatorsLimit,
            powerScale: _powerScale,
            permissionMode: PermissionMode.Collateral,
            minCrossMsgFee: _minCrossMsgFee,
            supplySource: SupplySourceHelper.native()
        });

        registrySubnetFacet.newSubnetActor(params);
    }

    // Test the updateReferenceSubnetContract method
    function test_UpdateReferenceSubnetContract() public {
        // Prepare new facet addresses and selector arrays
        address newGetterFacet = address(2); // Mocked new facet address
        address newManagerFacet = address(3); // Mocked new facet address
        bytes4[] memory newSubnetGetterSelectors = new bytes4[](1);
        newSubnetGetterSelectors[0] = 0x12345678; // Mocked selector
        bytes4[] memory newSubnetManagerSelectors = new bytes4[](1);
        newSubnetManagerSelectors[0] = 0x87654321; // Mocked selector

        registrySubnetGetterFacet.updateReferenceSubnetContract(
            newGetterFacet,
            newManagerFacet,
            newSubnetGetterSelectors,
            newSubnetManagerSelectors
        );

        // Validate the updates
        require(
            address(registrySubnetGetterFacet.getSubnetActorGetterFacet()) == newGetterFacet,
            "Getter facet address not updated correctly"
        );
        require(
            address(registrySubnetGetterFacet.getSubnetActorManagerFacet()) == newManagerFacet,
            "Manager facet address not updated correctly"
        );

        // Validate the updates for subnetGetterSelectors
        bytes4[] memory currentSubnetGetterSelectors = registrySubnetGetterFacet.getSubnetActorGetterSelectors();
        TestUtils.validateBytes4Array(
            currentSubnetGetterSelectors,
            newSubnetGetterSelectors,
            "SubnetGetterSelectors mismatch"
        );

        // Validate the updates for subnetManagerSelectors
        bytes4[] memory currentSubnetManagerSelectors = registrySubnetGetterFacet.getSubnetActorManagerSelectors();
        TestUtils.validateBytes4Array(
            currentSubnetManagerSelectors,
            newSubnetManagerSelectors,
            "SubnetManagerSelectors mismatch"
        );

        // Test only owner can update
        vm.prank(address(1)); // Set a different address as the sender
        vm.expectRevert(abi.encodeWithSelector(LibDiamond.NotOwner.selector)); // Expected revert message
        registrySubnetGetterFacet.updateReferenceSubnetContract(
            newGetterFacet,
            newManagerFacet,
            newSubnetGetterSelectors,
            newSubnetManagerSelectors
        );
    }
}
