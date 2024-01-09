// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.19;

import {StdAssertions} from "forge-std/StdAssertions.sol";
import {GatewayGetterFacet} from "../../src/gateway/GatewayGetterFacet.sol";
import {IntegrationTestBase, TestGatewayActor} from "../IntegrationTestBase.sol";

/// @title GatewayActor properties.
/// @dev It is suggested that all properties are defined here.
///     To check that a concrete GatewayActor instance holds the properties that target contract should inherit from this contract.
///     This contract must be abstract.
abstract contract GatewayActorBasicProperties is StdAssertions, TestGatewayActor {
    /// @notice The number of subnets is consistent within GatewayActor mechanisms.
    function invariant_GA_01_consistent_subnet_number() public virtual {
        assertEq(gwGetter.totalSubnets(), gwGetter.listSubnets().length, "the number of subnets is not consistent");
    }
}
