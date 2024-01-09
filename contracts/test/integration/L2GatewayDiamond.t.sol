// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.19;

import "forge-std/Test.sol";
import "../../src/errors/IPCErrors.sol";
import {EMPTY_BYTES, METHOD_SEND} from "../../src/constants/Constants.sol";
import {CrossMsg, StorableMsg} from "../../src/structs/CrossNet.sol";
import {FvmAddress} from "../../src/structs/FvmAddress.sol";
import {SubnetID, Subnet, IPCAddress, Validator} from "../../src/structs/Subnet.sol";
import {SubnetIDHelper} from "../../src/lib/SubnetIDHelper.sol";
import {FvmAddressHelper} from "../../src/lib/FvmAddressHelper.sol";
import {CrossMsgHelper} from "../../src/lib/CrossMsgHelper.sol";
import {GatewayDiamond, FEATURE_MULTILEVEL_CROSSMSG} from "../../src/GatewayDiamond.sol";
import {GatewayGetterFacet} from "../../src/gateway/GatewayGetterFacet.sol";
import {GatewayManagerFacet} from "../../src/gateway/GatewayManagerFacet.sol";
import {XnetMessagingFacet} from "../../src/gateway/router/XnetMessagingFacet.sol";
import {DiamondCutFacet} from "../../src/diamond/DiamondCutFacet.sol";
import {GatewayMessengerFacet} from "../../src/gateway/GatewayMessengerFacet.sol";
import {DiamondLoupeFacet} from "../../src/diamond/DiamondLoupeFacet.sol";
import {DiamondCutFacet} from "../../src/diamond/DiamondCutFacet.sol";
import {IntegrationTestBase} from "../IntegrationTestBase.sol";
import {L2GatewayActorDiamond} from "../IntegrationTestPresets.sol";
import {FilAddress} from "fevmate/utils/FilAddress.sol";

contract L2GatewayActorDiamondTest is Test, L2GatewayActorDiamond {
    using SubnetIDHelper for SubnetID;
    using CrossMsgHelper for CrossMsg;

    function testGatewayDiamond_CommitParentFinality_BigNumberOfMessages() public {
        uint256 n = 2000;
        FvmAddress[] memory validators = new FvmAddress[](1);
        validators[0] = FvmAddressHelper.from(vm.addr(100));
        vm.deal(vm.addr(100), 1);

        uint256[] memory weights = new uint[](1);
        weights[0] = 100;

        SubnetID memory id = gwGetter.getNetworkName();

        CrossMsg[] memory topDownMsgs = new CrossMsg[](n);
        for (uint64 i = 0; i < n; i++) {
            topDownMsgs[i] = CrossMsg({
                message: StorableMsg({
                    from: IPCAddress({subnetId: id, rawAddress: FvmAddressHelper.from(address(this))}),
                    to: IPCAddress({subnetId: id, rawAddress: FvmAddressHelper.from(address(this))}),
                    value: 0,
                    nonce: i,
                    method: this.callback.selector,
                    params: EMPTY_BYTES,
                    fee: DEFAULT_CROSS_MSG_FEE
                }),
                wrapped: false
            });
        }

        vm.startPrank(FilAddress.SYSTEM_ACTOR);

        gwXnetMessagingFacet.applyCrossMessages(topDownMsgs);
        require(gwGetter.getSubnetTopDownMsgsLength(id) == 0, "unexpected top-down message");
        (bool ok, uint64 tdn) = gwGetter.getAppliedTopDownNonce(id);
        require(!ok && tdn == 0, "unexpected nonce");

        vm.stopPrank();
    }

    function testGatewayDiamond_Propagate_Works_WithFeeRemainderNew() external {
        if (!FEATURE_MULTILEVEL_CROSSMSG) {
            // skip
            return;
        }
        (, address[] memory validators) = setupValidators();
        address caller = validators[0];

        bytes32 postboxId = setupWhiteListMethod(caller);

        vm.deal(caller, 1 ether);

        vm.expectCall(caller, 1 ether - gwGetter.crossMsgFee(), new bytes(0), 1);
        vm.prank(caller);
        gwMessenger.propagate{value: 1 ether}(postboxId);

        require(caller.balance == 1 ether - gwGetter.crossMsgFee(), "unexpected balance");
    }

    function testGatewayDiamond_Propagate_Works_NoFeeReminder() external {
        if (!FEATURE_MULTILEVEL_CROSSMSG) {
            // skip
            return;
        }
        (, address[] memory validators) = setupValidators();
        address caller = validators[0];

        uint256 fee = gwGetter.crossMsgFee();

        bytes32 postboxId = setupWhiteListMethod(caller);

        vm.deal(caller, fee);

        vm.prank(caller);
        vm.expectCall(caller, 0, EMPTY_BYTES, 0);
        gwMessenger.propagate{value: fee}(postboxId);
        require(caller.balance == 0, "unexpected balance");
    }

    function testGatewayDiamond_Propagate_Fails_NotEnoughFee() public {
        if (!FEATURE_MULTILEVEL_CROSSMSG) {
            // skip
            return;
        }
        address caller = vm.addr(100);
        vm.deal(caller, 1 ether);

        vm.expectRevert(NotEnoughFee.selector);
        gwMessenger.propagate(bytes32(""));
    }

    function setupWhiteListMethod(address caller) internal returns (bytes32) {
        registerSubnet(DEFAULT_COLLATERAL_AMOUNT, address(this));

        CrossMsg memory crossMsg = CrossMsg({
            message: StorableMsg({
                from: IPCAddress({
                    subnetId: gwGetter.getNetworkName().createSubnetId(caller),
                    rawAddress: FvmAddressHelper.from(caller)
                }),
                to: IPCAddress({
                    subnetId: gwGetter.getNetworkName().createSubnetId(address(this)),
                    rawAddress: FvmAddressHelper.from(address(this))
                }),
                value: DEFAULT_CROSS_MSG_FEE + 1,
                nonce: 0,
                method: METHOD_SEND,
                params: new bytes(0),
                fee: DEFAULT_CROSS_MSG_FEE
            }),
            wrapped: false
        });
        CrossMsg[] memory msgs = new CrossMsg[](1);
        msgs[0] = crossMsg;

        vm.prank(FilAddress.SYSTEM_ACTOR);
        gwXnetMessagingFacet.applyCrossMessages(msgs);

        return crossMsg.toHash();
    }

    function callback() public view {}
}
