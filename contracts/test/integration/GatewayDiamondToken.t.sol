// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.19;

import "forge-std/Test.sol";

import "../../src/errors/IPCErrors.sol";
import {EMPTY_BYTES, METHOD_SEND, EMPTY_HASH} from "../../src/constants/Constants.sol";
import {CrossMsg, BottomUpMsgBatch, StorableMsg} from "../../src/structs/CrossNet.sol";
import {FvmAddress} from "../../src/structs/FvmAddress.sol";
import {SubnetID, Subnet, SupplySource, SupplyKind, Validator} from "../../src/structs/Subnet.sol";
import {SubnetIDHelper} from "../../src/lib/SubnetIDHelper.sol";
import {FvmAddressHelper} from "../../src/lib/FvmAddressHelper.sol";
import {CrossMsgHelper} from "../../src/lib/CrossMsgHelper.sol";
import {SupplySourceHelper} from "../../src/lib/SupplySourceHelper.sol";
import {StorableMsgHelper} from "../../src/lib/StorableMsgHelper.sol";
import {FilAddress} from "fevmate/utils/FilAddress.sol";
import {GatewayDiamond} from "../../src/GatewayDiamond.sol";
import {LibGateway} from "../../src/lib/LibGateway.sol";
import {TestUtils} from "../helpers/TestUtils.sol";
import {IntegrationTestBase} from "../IntegrationTestBase.sol";
import {SubnetActorDiamond} from "../../src/SubnetActorDiamond.sol";
import {GatewayGetterFacet} from "../../src/gateway/GatewayGetterFacet.sol";
import {GatewayMessengerFacet} from "../../src/gateway/GatewayMessengerFacet.sol";
import {GatewayManagerFacet} from "../../src/gateway/GatewayManagerFacet.sol";
import {SubnetActorManagerFacet} from "../../src/subnet/SubnetActorManagerFacet.sol";
import {SubnetActorGetterFacet} from "../../src/subnet/SubnetActorGetterFacet.sol";
import {DiamondLoupeFacet} from "../../src/diamond/DiamondLoupeFacet.sol";
import {DiamondCutFacet} from "../../src/diamond/DiamondCutFacet.sol";

import {IERC20} from "openzeppelin-contracts/token/ERC20/IERC20.sol";
import {ERC20PresetFixedSupply} from "../helpers/ERC20PresetFixedSupply.sol";
import {IERC20Errors} from "openzeppelin-contracts/interfaces/draft-IERC6093.sol";

contract GatewayDiamondTokenTest is Test, IntegrationTestBase {
    using SubnetIDHelper for SubnetID;
    using CrossMsgHelper for CrossMsg;
    using StorableMsgHelper for StorableMsg;
    using FvmAddressHelper for FvmAddress;

    IERC20 private token;

    function setUp() public override {
        super.setUp();

        token = new ERC20PresetFixedSupply("TestToken", "TEST", 1_000_000, address(this));
    }

    function test_fundWithToken_NativeSupply_Reverts() public {
        (address validatorAddress, bytes memory publicKey) = TestUtils.deriveValidatorAddress(100);
        join(validatorAddress, publicKey);

        address caller = vm.addr(1);
        vm.deal(caller, 100);

        (SubnetID memory subnetId, , , , ) = getSubnet(address(saManager));

        vm.prank(caller);
        vm.expectRevert(SupplySourceHelper.UnexpectedSupplySource.selector);
        gwManager.fundWithToken(subnetId, FvmAddressHelper.from(caller), 100);
    }

    function test_fund_TokenSupply_Reverts() public {
        address caller = vm.addr(1);
        vm.deal(caller, 100);

        Subnet memory subnet = createTokenSubnet(address(token));

        vm.prank(caller);
        vm.expectRevert(SupplySourceHelper.UnexpectedSupplySource.selector);
        gwManager.fund{value: 100}(subnet.id, FvmAddressHelper.from(caller));
    }

    function testFail_InexistentToken() public {
        // Reverts because the token doesn't exist at that address.
        address addr = vm.addr(999);
        createTokenSubnet(addr);
    }

    function test_fundWithToken_FailsInsufficientBalance() public {
        Subnet memory subnet = createTokenSubnet(address(token));

        // account has native balance but no tokens, reverts.
        address caller = vm.addr(1);
        vm.deal(caller, 100);
        vm.prank(caller);
        vm.expectRevert(
            abi.encodeWithSelector(IERC20Errors.ERC20InsufficientAllowance.selector, address(gatewayDiamond), 0, 1)
        );
        gwManager.fundWithToken(subnet.id, FvmAddressHelper.from(caller), 1);
    }

    function test_fundWithToken() public {
        Subnet memory subnet = createTokenSubnet(address(token));

        address caller = vm.addr(1);
        vm.deal(caller, 100);
        token.transfer(caller, 100);
        vm.startPrank(caller);

        // Caller approves the gateway to spend funds on their behalf.
        token.approve(address(gatewayDiamond), 10);

        // Funding succeeds and the right event is emitted.
        CrossMsg memory expected = CrossMsgHelper.createFundMsg(
            subnet.id,
            caller,
            FvmAddressHelper.from(caller),
            10,
            0
        );
        vm.expectEmit(true, true, true, true, address(gatewayDiamond));
        emit LibGateway.NewTopDownMessage(address(saDiamond), expected);
        gwManager.fundWithToken(subnet.id, FvmAddressHelper.from(caller), 10);

        // Assert post-conditions.
        (, Subnet memory subnetAfter) = gwGetter.getSubnet(subnet.id);
        assertEq(subnetAfter.circSupply, 10);
        assertEq(subnetAfter.topDownNonce, 1);

        // A new funding attempt with exhausted token balance should fail.
        vm.expectRevert();
        gwManager.fundWithToken(subnet.id, FvmAddressHelper.from(caller), 10);

        // And the subnet's state should not have been updated.
        (, subnetAfter) = gwGetter.getSubnet(subnet.id);
        assertEq(subnetAfter.circSupply, 10);
        assertEq(subnetAfter.topDownNonce, 1);

        // After topping up it succeeds again.
        token.approve(address(gatewayDiamond), 5);
        gwManager.fundWithToken(subnet.id, FvmAddressHelper.from(caller), 5);

        // And the subnet's bookkeeping is correct.
        (, subnetAfter) = gwGetter.getSubnet(subnet.id);
        assertEq(subnetAfter.circSupply, 15);
        assertEq(subnetAfter.topDownNonce, 2);
    }

    event Transfer(address indexed from, address indexed to, uint256 value);

    function test_withdrawToken_Parent() public {
        Subnet memory subnet = createTokenSubnet(address(token));

        // Fund an account in the subnet.
        address caller = vm.addr(1);
        token.transfer(caller, 100);
        vm.prank(caller);
        token.approve(address(gatewayDiamond), 15);
        vm.prank(caller);
        gwManager.fundWithToken(subnet.id, FvmAddressHelper.from(caller), 15);

        // Now create a new recipient on the parent.
        address recipient = vm.addr(42);

        // Commit the withdrawal message on the parent.
        CrossMsg[] memory msgs = new CrossMsg[](1);
        uint256 value = 8;
        msgs[0] = CrossMsgHelper.createReleaseMsg(subnet.id, caller, FvmAddressHelper.from(recipient), value, 0);

        BottomUpMsgBatch memory batch = BottomUpMsgBatch({
            subnetID: subnet.id,
            blockHeight: gwGetter.bottomUpMsgBatchPeriod(),
            msgs: msgs
        });

        vm.prank(address(saDiamond));
        vm.expectEmit(true, true, true, true, address(token));
        emit Transfer(address(gatewayDiamond), recipient, value);
        gwBottomUpRouterFacet.execBottomUpMsgBatch(batch);

        // Assert post-conditions.
        (, Subnet memory subnetAfter) = gwGetter.getSubnet(subnet.id);
        assertEq(subnetAfter.circSupply, 7);
        assertEq(subnetAfter.topDownNonce, 1);
        assertEq(subnetAfter.appliedBottomUpNonce, 1);

        // Now attempt to withdraw beyond the circulating supply.
        // This would be a malicious message.
        batch.msgs[0].message.value = 10;

        // This reverts.
        vm.prank(address(saDiamond));
        vm.expectRevert();
        gwBottomUpRouterFacet.execBottomUpMsgBatch(batch);
    }

    function test_childToParentCall() public {
        Subnet memory subnet = createTokenSubnet(address(token));

        // Fund an account in the subnet.
        address caller = vm.addr(1);
        token.transfer(caller, 100);
        vm.prank(caller);
        token.approve(address(gatewayDiamond), 15);
        vm.prank(caller);
        gwManager.fundWithToken(subnet.id, FvmAddressHelper.from(caller), 15);

        // Now create a new recipient on the parent.
        address recipient = vm.addr(42);

        // Commit a xnet message that isn't a simple bare transfer.
        CrossMsg[] memory msgs = new CrossMsg[](1);
        uint256 value = 8;
        msgs[0] = CrossMsgHelper.createReleaseMsg(subnet.id, caller, FvmAddressHelper.from(recipient), value, 0);
        msgs[0].message.method = bytes4(0x11223344);
        msgs[0].message.params = bytes("hello");

        BottomUpMsgBatch memory batch = BottomUpMsgBatch({
            subnetID: subnet.id,
            blockHeight: gwGetter.bottomUpMsgBatchPeriod(),
            msgs: msgs
        });

        // Verify that we received the call and that the recipient has the tokens.
        vm.prank(address(saDiamond));
        vm.etch(recipient, bytes("foo")); // set some code at the destination address to trick Solidity into calling the contract.
        vm.expectCall(recipient, bytes.concat(bytes4(0x11223344), bytes("hello")));
        gwBottomUpRouterFacet.execBottomUpMsgBatch(batch);
        assertEq(token.balanceOf(recipient), 8);
    }

    function test_propagation() public {
        // TODO:
        // 1. Test that propagation is rejected when sender is ERC20.
        // 2. Test that propagation is rejected when receiver is ERC20.
        // 3. Test that propagation is rejected when an intermediary subnet is ERC20.
    }

    function createTokenSubnet(address tokenAddress) internal returns (Subnet memory) {
        // Create a subnet actor in the root network, with an ERC20 supply source with the specified token address.
        SubnetActorDiamond.ConstructorParams memory saConstructorParams = defaultSubnetActorParamsWithGateway(
            address(gatewayDiamond)
        );
        saConstructorParams.supplySource = SupplySource({kind: SupplyKind.ERC20, tokenAddress: tokenAddress});

        // Override the state variables with the new subnet.
        saDiamond = createSubnetActor(saConstructorParams);
        saManager = SubnetActorManagerFacet(address(saDiamond));
        saGetter = SubnetActorGetterFacet(address(saDiamond));
        saLouper = DiamondLoupeFacet(address(saDiamond));
        saCutter = DiamondCutFacet(address(saDiamond));

        addValidator(TOPDOWN_VALIDATOR_1, 100);

        (address validatorAddress, bytes memory publicKey) = TestUtils.deriveValidatorAddress(100);
        join(validatorAddress, publicKey);

        SubnetID memory subnetId = gwGetter.getNetworkName().createSubnetId(address(saDiamond));

        (bool exists, Subnet memory subnet) = gwGetter.getSubnet(subnetId);
        assert(exists);
        return subnet;
    }
}
