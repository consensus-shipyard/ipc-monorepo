// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.19;

import {MethodNotAllowed} from "../errors/IPCErrors.sol";
import {IRelayerRewardDistributor} from "../interfaces/ISubnetActor.sol";
import {QuorumObjKind} from "../structs/Quorum.sol";
import {Pausable} from "../lib/LibPausable.sol";
import {ReentrancyGuard} from "../lib/LibReentrancyGuard.sol";
import {SubnetActorModifiers} from "../lib/LibSubnetActorStorage.sol";
import {LibStaking} from "../lib/LibStaking.sol";
import {LibSubnetActor} from "../lib/LibSubnetActor.sol";

contract SubnetActorRewardFacet is IRelayerRewardDistributor, SubnetActorModifiers, ReentrancyGuard, Pausable {
    /// @notice Validator claims their released collateral.
    function claim() external nonReentrant whenNotPaused {
        LibStaking.claimCollateral(msg.sender);
    }

    /// @notice Relayer claims its reward.
    function claimRewardForRelayer() external nonReentrant whenNotPaused {
        LibStaking.claimRewardForRelayer(msg.sender);
    }

    /// @notice Distributes rewards to relayers for a specific checkpoint.
    /// @param height The height of the checkpoint for which rewards are being distributed.
    /// @param reward The total amount of reward to be distributed.
    /// @param kind The type of object for which rewards are being distributed.
    function distributeRewardToRelayers(
        uint256 height,
        uint256 reward,
        QuorumObjKind kind
    ) external payable whenNotPaused onlyGateway {
        if (reward == 0) {
            return;
        }

        // get rewarded addresses
        address[] memory relayers = new address[](0);
        if (kind == QuorumObjKind.Checkpoint) {
            relayers = LibSubnetActor.checkpointRewardedAddrs(height);
        } else if (kind == QuorumObjKind.BottomUpMsgBatch) {
            // FIXME: The distribution of rewards for batches can't be done
            // as for checkpoints (due to how they are submitted). As
            // we are running out of time, we'll defer this for the future.
            revert MethodNotAllowed("rewards not defined for batches");
        } else {
            revert MethodNotAllowed("rewards not defined for object kind");
        }

        // comupte reward
        // we are not distributing equally, this logic should be decoupled
        // into different reward policies.
        uint256 relayersLength = relayers.length;
        if (relayersLength == 0) {
            return;
        }
        if (reward < relayersLength) {
            return;
        }
        uint256 relayerReward = reward / relayersLength;

        // distribute reward
        for (uint256 i; i < relayersLength; ) {
            s.relayerRewards.rewards[relayers[i]] += relayerReward;
            unchecked {
                ++i;
            }
        }
    }
}
