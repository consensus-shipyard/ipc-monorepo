// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.19;

import {InvalidBatchEpoch, MaxMsgsPerBatchExceeded, BatchWithNoMessages, InvalidSignatureErr, InvalidCheckpointEpoch} from "../errors/IPCErrors.sol";
import {IGateway} from "../interfaces/IGateway.sol";
import {BottomUpCheckpoint, BottomUpMsgBatch, BottomUpMsgBatchInfo} from "../structs/CrossNet.sol";
import {Validator, ValidatorSet} from "../structs/Subnet.sol";
import {MultisignatureChecker} from "../lib/LibMultisignatureChecker.sol";
import {ReentrancyGuard} from "../lib/LibReentrancyGuard.sol";
import {SubnetActorModifiers} from "../lib/LibSubnetActorStorage.sol";
import {LibValidatorSet, LibStaking} from "../lib/LibStaking.sol";
import {EnumerableSet} from "openzeppelin-contracts/utils/structs/EnumerableSet.sol";
import {LibSubnetActor} from "../lib/LibSubnetActor.sol";
import {Pausable} from "../lib/LibPausable.sol";

contract SubnetActorCheckpointingFacet is SubnetActorModifiers, ReentrancyGuard, Pausable {
    using EnumerableSet for EnumerableSet.AddressSet;
    using LibValidatorSet for ValidatorSet;

    /// @notice Submits a checkpoint commitment for execution.
    ///  @dev   It triggers the commitment of the checkpoint and any other side-effects that
    ///         need to be triggered by the checkpoint such as relayer reward book keeping.
    /// @param checkpoint The executed bottom-up checkpoint.
    /// @param signatories The addresses of validators signing the checkpoint.
    /// @param signatures The signatures of validators on the checkpoint.
    function submitCheckpoint(
        BottomUpCheckpoint calldata checkpoint,
        address[] calldata signatories,
        bytes[] calldata signatures
    ) external whenNotPaused {
        // the checkpoint height must be equal to the last bottom-up checkpoint height or
        // the next one
        if (
            checkpoint.blockHeight != s.lastBottomUpCheckpointHeight + s.bottomUpCheckPeriod &&
            checkpoint.blockHeight != s.lastBottomUpCheckpointHeight
        ) {
            revert InvalidCheckpointEpoch();
        }
        bytes32 checkpointHash = keccak256(abi.encode(checkpoint));

        if (checkpoint.blockHeight == s.lastBottomUpCheckpointHeight + s.bottomUpCheckPeriod) {
            // validate signatures and quorum threshold, revert if validation fails
            validateActiveQuorumSignatures({signatories: signatories, hash: checkpointHash, signatures: signatures});

            // If the checkpoint height is the next expected height then this is a new checkpoint which must be executed
            // in the Gateway Actor, the checkpoint and the relayer must be stored, last bottom-up checkpoint updated.
            s.committedCheckpoints[checkpoint.blockHeight] = checkpoint;

            // slither-disable-next-line unused-return
            s.relayerRewards.checkpointRewarded[checkpoint.blockHeight].add(msg.sender);

            s.lastBottomUpCheckpointHeight = checkpoint.blockHeight;

            // Commit in gateway to distribute rewards
            IGateway(s.ipcGatewayAddr).commitCheckpoint(checkpoint);

            // confirming the changes in membership in the child
            LibStaking.confirmChange(checkpoint.nextConfigurationNumber);
        } else if (checkpoint.blockHeight == s.lastBottomUpCheckpointHeight) {
            // If the checkpoint height is equal to the last checkpoint height, then this is a repeated submission.
            // We should store the relayer, but not to execute checkpoint again.
            // In this case, we do not verify the signatures for this checkpoint again,
            // but we add the relayer to the list of all relayers for this checkpoint to be rewarded later.
            // The reason for comparing hashes instead of verifying signatures is the following:
            // once the checkpoint is executed, the active validator set changes
            // and can only be used to validate the next checkpoint, not another instance of the last one.
            bytes32 lastCheckpointHash = keccak256(abi.encode(s.committedCheckpoints[checkpoint.blockHeight]));
            if (checkpointHash == lastCheckpointHash) {
                // slither-disable-next-line unused-return
                s.relayerRewards.checkpointRewarded[checkpoint.blockHeight].add(msg.sender);
            }
        }
    }

    /// @notice Submits a batch of bottom-up messages for execution.
    /// @dev It triggers the execution of a cross-net message batch.
    /// @param batch The batch of bottom-up messages.
    /// @param signatories The addresses of validators signing the batch.
    /// @param signatures The signatures of validators on the batch.
    function submitBottomUpMsgBatch(
        BottomUpMsgBatch calldata batch,
        address[] calldata signatories,
        bytes[] calldata signatures
    ) external {
        // forbid the submission of batches from the past
        if (batch.blockHeight < s.lastBottomUpBatch.blockHeight) {
            revert InvalidBatchEpoch();
        }
        if (batch.msgs.length > s.maxMsgsPerBottomUpBatch) {
            revert MaxMsgsPerBatchExceeded();
        }
        // if the batch height is not max, we only supoprt batch submission in period epochs
        if (batch.msgs.length != s.maxMsgsPerBottomUpBatch && batch.blockHeight % s.bottomUpMsgBatchPeriod != 0) {
            revert InvalidBatchEpoch();
        }
        if (batch.msgs.length == 0) {
            revert BatchWithNoMessages();
        }

        bytes32 batchHash = keccak256(abi.encode(batch));

        if (batch.blockHeight == s.lastBottomUpBatch.blockHeight) {
            // If the batch info is equal to the last batch info, then this is a repeated submission.
            // We should store the relayer, but not to execute batch again following the same reward logic
            // used for checkpoints.
            if (batchHash == s.lastBottomUpBatch.hash) {
                // slither-disable-next-line unused-return
                s.relayerRewards.batchRewarded[batch.blockHeight].add(msg.sender);
            }
        } else {
            // validate signatures and quorum threshold, revert if validation fails
            validateActiveQuorumSignatures({signatories: signatories, hash: batchHash, signatures: signatures});

            // If the checkpoint height is the next expected height then this is a new batch,
            // and should be forwarded to the gateway for execution.
            s.lastBottomUpBatch = BottomUpMsgBatchInfo({blockHeight: batch.blockHeight, hash: batchHash});

            // slither-disable-next-line unused-return
            s.relayerRewards.batchRewarded[batch.blockHeight].add(msg.sender);

            // Execute messages.
            IGateway(s.ipcGatewayAddr).execBottomUpMsgBatch(batch);
        }
    }

    /// @notice Checks whether the signatures are valid for the provided signatories and hash within the current validator set.
    ///         Reverts otherwise.
    /// @dev Signatories in `signatories` and their signatures in `signatures` must be provided in the same order.
    ///       Having it public allows external users to perform sanity-check verification if needed.
    /// @param signatories The addresses of the signatories.
    /// @param hash The hash of the checkpoint.
    /// @param signatures The packed signatures of the checkpoint.
    function validateActiveQuorumSignatures(
        address[] memory signatories,
        bytes32 hash,
        bytes[] memory signatures
    ) public view {
        // This call reverts if at least one of the signatories (validator) is not in the active validator set.
        uint256[] memory collaterals = s.validatorSet.getTotalPowerOfValidators(signatories);
        uint256 activeCollateral = s.validatorSet.getTotalActivePower();

        uint256 threshold = (activeCollateral * s.majorityPercentage) / 100;

        (bool valid, MultisignatureChecker.Error err) = MultisignatureChecker.isValidWeightedMultiSignature({
            signatories: signatories,
            weights: collaterals,
            threshold: threshold,
            hash: hash,
            signatures: signatures
        });

        if (!valid) {
            revert InvalidSignatureErr(uint8(err));
        }
    }
}
