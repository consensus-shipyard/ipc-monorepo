// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.19;

import {IRelayerRewardDistributor} from "../../interfaces/ISubnetActor.sol";
import {GatewayActorModifiers} from "../../lib/LibGatewayActorStorage.sol";
import {BottomUpMsgBatch} from "../../structs/CrossNet.sol";
import {LibGateway} from "../../lib/LibGateway.sol";
import {BatchNotCreated, BatchAlreadyExists, InvalidBatchEpoch, NotEnoughSubnetCircSupply, SubnetNotActive, SubnetNotFound, InvalidBatchSource, MaxMsgsPerBatchExceeded, BatchWithNoMessages, InvalidCrossMsgDstSubnet, NotRegisteredSubnet, InvalidCrossMsgNonce} from "../../errors/IPCErrors.sol";
import {Subnet} from "../../structs/Subnet.sol";
import {LibQuorum} from "../../lib/LibQuorum.sol";
import {QuorumObjKind} from "../../structs/Quorum.sol";
import {Address} from "openzeppelin-contracts/utils/Address.sol";
import {IPCMsgType} from "../../enums/IPCMsgType.sol";

import {CrossMsg, SubnetID} from "../../structs/CrossNet.sol";
import {CrossMsgHelper} from "../../lib/CrossMsgHelper.sol";

import {SupplySourceHelper} from "../../lib/SupplySourceHelper.sol";
import {SupplySource} from "../../structs/Subnet.sol";
import {SubnetActorGetterFacet} from "../../subnet/SubnetActorGetterFacet.sol";

import {SubnetIDHelper} from "../../lib/SubnetIDHelper.sol";
import {StorableMsgHelper} from "../../lib/StorableMsgHelper.sol";
import {StorableMsg} from "../../structs/CrossNet.sol";

contract BottomUpRouterFacet is GatewayActorModifiers {
    using SubnetIDHelper for SubnetID;
    using CrossMsgHelper for CrossMsg;
    using SupplySourceHelper for SupplySource;
    using StorableMsgHelper for StorableMsg;

    /// @notice submit a batch of cross-net messages for execution.
    /// @dev this method is called by the corresponding subnet actor.
    /// Called from a subnet actor if the batch is valid.
    /// @param batch The batch of bottom-up cross-network messages to be executed.
    function execBottomUpMsgBatch(BottomUpMsgBatch calldata batch) external {
        if (batch.subnetID.getActor() != msg.sender) {
            revert InvalidBatchSource();
        }

        LibGateway.checkMsgLength(batch);

        (bool subnetExists, Subnet storage subnet) = LibGateway.getSubnet(msg.sender);
        if (!subnetExists) {
            revert SubnetNotFound();
        }

        uint256 totalValue;
        uint256 totalFee;
        uint256 crossMsgLength = batch.msgs.length;
        for (uint256 i; i < crossMsgLength; ) {
            totalValue += batch.msgs[i].message.value;
            totalFee += batch.msgs[i].message.fee;
            unchecked {
                ++i;
            }
        }

        uint256 totalAmount = totalFee + totalValue;

        if (subnet.circSupply < totalAmount) {
            revert NotEnoughSubnetCircSupply();
        }

        subnet.circSupply -= totalAmount;

        // execute cross-messages
        LibGateway.applyMessages(subnet.id, batch.msgs);

        if (s.crossMsgRelayerRewards) {
            // reward relayers in the subnet for committing the previous checkpoint
            // slither-disable-next-line unused-return
            Address.functionCallWithValue({
                target: msg.sender,
                data: abi.encodeCall(
                    IRelayerRewardDistributor.distributeRewardToRelayers,
                    (block.number, totalFee, QuorumObjKind.Checkpoint)
                ),
                value: totalFee
            });
        }
    }

    /// @notice cuts a new message batch if the batch period is reached without
    /// the maximum number of messages being reached.
    /// @param batch - a bottom-up batch
    /// @param membershipRootHash - a root hash of the Merkle tree built from the validator public keys and their weight
    /// @param membershipWeight - the total weight of the membership
    function createBottomUpMsgBatch(
        BottomUpMsgBatch calldata batch,
        bytes32 membershipRootHash,
        uint256 membershipWeight
    ) external systemActorOnly {
        LibGateway.checkMsgLength(batch);
        // We only externally trigger new batches if the maximum number
        // of messages for the batch hasn't been reached.
        // We also check that we are not trying to create a batch from
        // the future
        if (batch.blockHeight % s.bottomUpMsgBatchPeriod != 0 || block.number <= batch.blockHeight) {
            revert InvalidBatchEpoch();
        }

        if (LibGateway.bottomUpBatchMsgsExists(batch.blockHeight)) {
            revert BatchAlreadyExists();
        }

        LibQuorum.createQuorumInfo({
            self: s.bottomUpMsgBatchQuorumMap,
            objHeight: batch.blockHeight,
            objHash: keccak256(abi.encode(batch)),
            membershipRootHash: membershipRootHash,
            membershipWeight: membershipWeight,
            majorityPercentage: s.majorityPercentage
        });
        LibGateway.storeBottomUpMsgBatch(batch);
    }

    /// @notice Set a new batch retention height and garbage collect all batches in range [`retentionHeight`, `newRetentionHeight`)
    /// @param newRetentionHeight - the height of the oldest batch to keep
    function pruneBottomUpMsgBatches(uint256 newRetentionHeight) external systemActorOnly {
        for (uint256 h = s.bottomUpMsgBatchQuorumMap.retentionHeight; h < newRetentionHeight; ) {
            delete s.bottomUpMsgBatches[h];
            unchecked {
                ++h;
            }
        }

        LibQuorum.pruneQuorums(s.bottomUpMsgBatchQuorumMap, newRetentionHeight);
    }

    /// @notice checks whether the provided batch signature for the block at height `height` is valid and accumulates that
    /// @param height - the height of the block in the checkpoint
    /// @param membershipProof - a Merkle proof that the validator was in the membership at height `height` with weight `weight`
    /// @param weight - the weight of the validator
    /// @param signature - the signature of the checkpoint
    function addBottomUpMsgBatchSignature(
        uint256 height,
        bytes32[] memory membershipProof,
        uint256 weight,
        bytes memory signature
    ) external {
        LibQuorum.isHeightAlreadyProcessed(s.bottomUpMsgBatchQuorumMap, height);

        // slither-disable-next-line unused-return
        (bool exists, ) = LibGateway.getBottomUpMsgBatch(height);
        if (!exists) {
            revert BatchNotCreated();
        }
        LibQuorum.addQuorumSignature({
            self: s.bottomUpMsgBatchQuorumMap,
            height: height,
            membershipProof: membershipProof,
            weight: weight,
            signature: signature
        });
    }
}
