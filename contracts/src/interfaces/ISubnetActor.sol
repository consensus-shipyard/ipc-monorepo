// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.19;

import {BottomUpCheckpoint} from "../structs/CrossNet.sol";
import {QuorumObjKind} from "../structs/Quorum.sol";

interface IRelayerRewardDistributor {
    /// reward the relayers for processing checkpoint at height `height`.
    /// The reword includes the fixed reward for a relayer defined in the contract and `amount` of fees from the cross-messages.
    function distributeRewardToRelayers(uint256 height, uint256 amount, QuorumObjKind kind) external payable;
}
