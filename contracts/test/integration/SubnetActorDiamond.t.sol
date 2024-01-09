// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.19;

import "../../src/errors/IPCErrors.sol";
import {Test} from "forge-std/Test.sol";
import "forge-std/console.sol";
import {TestUtils} from "../helpers/TestUtils.sol";
import {SelectorLibrary} from "../helpers/SelectorLibrary.sol";
import {NumberContractFacetSeven} from "../helpers/NumberContractFacetSeven.sol";
import {NumberContractFacetEight} from "../helpers/NumberContractFacetEight.sol";
import {METHOD_SEND} from "../../src/constants/Constants.sol";
import {ConsensusType} from "../../src/enums/ConsensusType.sol";
import {BottomUpMsgBatch, CrossMsg, BottomUpCheckpoint, StorableMsg} from "../../src/structs/CrossNet.sol";
import {FvmAddress} from "../../src/structs/FvmAddress.sol";
import {SubnetID, PermissionMode, IPCAddress, Subnet, SupplySource, ValidatorInfo} from "../../src/structs/Subnet.sol";
import {IERC165} from "../../src/interfaces/IERC165.sol";
import {IGateway} from "../../src/interfaces/IGateway.sol";
import {IDiamond} from "../../src/interfaces/IDiamond.sol";
import {IDiamondCut} from "../../src/interfaces/IDiamondCut.sol";
import {IDiamondLoupe} from "../../src/interfaces/IDiamondLoupe.sol";
import {FvmAddressHelper} from "../../src/lib/FvmAddressHelper.sol";
import {MultisignatureChecker} from "../../src/lib/LibMultisignatureChecker.sol";
import {StorableMsgHelper} from "../../src/lib/StorableMsgHelper.sol";
import {SubnetIDHelper} from "../../src/lib/SubnetIDHelper.sol";
import {SubnetActorDiamond, FunctionNotFound} from "../../src/SubnetActorDiamond.sol";
import {FEATURE_CHECKPOINT_RELAYER_REWARDS} from "../../src/GatewayDiamond.sol";
import {SubnetActorManagerFacet} from "../../src/subnet/SubnetActorManagerFacet.sol";
import {SubnetActorGetterFacet} from "../../src/subnet/SubnetActorGetterFacet.sol";
import {SubnetActorPauseFacet} from "../../src/subnet/SubnetActorPauseFacet.sol";
import {SubnetActorCheckpointingFacet} from "../../src/subnet/SubnetActorCheckpointingFacet.sol";
import {SubnetActorRewardFacet} from "../../src/subnet/SubnetActorRewardFacet.sol";
import {DiamondCutFacet} from "../../src/diamond/DiamondCutFacet.sol";
import {FilAddress} from "fevmate/utils/FilAddress.sol";
import {LibStaking} from "../../src/lib/LibStaking.sol";
import {LibDiamond} from "../../src/lib/LibDiamond.sol";
import {Pausable} from "../../src/lib/LibPausable.sol";
import {SupplySourceHelper} from "../../src/lib/SupplySourceHelper.sol";

import {IntegrationTestBase} from "../IntegrationTestBase.sol";

contract SubnetActorDiamondTest is Test, IntegrationTestBase {
    using SubnetIDHelper for SubnetID;
    using FilAddress for address;
    using FvmAddressHelper for FvmAddress;

    address gatewayAddress;

    function setUp() public override {
        super.setUp();

        gatewayAddress = address(gatewayDiamond);
    }

    function testSubnetActorDiamond_NewSubnetActorWithDefaultParams() public view {
        SubnetID memory _parentId = SubnetID(ROOTNET_CHAINID, new address[](0));
        SubnetActorDiamond.ConstructorParams memory params = defaultSubnetActorParamsWithGateway(
            address(gatewayDiamond)
        );

        require(saGetter.ipcGatewayAddr() == params.ipcGatewayAddr, "unexpected gateway");
        require(saGetter.minActivationCollateral() == params.minActivationCollateral, "unexpected collateral");
        require(saGetter.minValidators() == params.minValidators, "unexpected minValidators");
        require(saGetter.consensus() == params.consensus, "unexpected consensus");
        require(saGetter.getParent().equals(_parentId), "unexpected parent");
        require(saGetter.activeValidatorsLimit() == 100, "unexpected activeValidatorsLimit");
        require(saGetter.powerScale() == params.powerScale, "unexpected powerscale");
        require(saGetter.minCrossMsgFee() == DEFAULT_CROSS_MSG_FEE, "unexpected cross-msg fee");
        require(saGetter.bottomUpCheckPeriod() == params.bottomUpCheckPeriod, "unexpected bottom-up period");
        require(saGetter.majorityPercentage() == params.majorityPercentage, "unexpected majority percentage");
        require(saGetter.getParent().toHash() == _parentId.toHash(), "unexpected parent subnetID hash");
    }

    function testSubnetActorDiamondReal_LoupeFunction() public view {
        require(saLouper.facets().length == 7, "unexpected length");
        require(saLouper.supportsInterface(type(IERC165).interfaceId) == true, "IERC165 not supported");
        require(saLouper.supportsInterface(type(IDiamondCut).interfaceId) == true, "IDiamondCut not supported");
        require(saLouper.supportsInterface(type(IDiamondLoupe).interfaceId) == true, "IDiamondLoupe not supported");
    }

    /// @notice Testing the basic join, stake, leave lifecycle of validators
    function testSubnetActorDiamond_BasicLifeCycle() public {
        (address validator1, uint256 privKey1, bytes memory publicKey1) = TestUtils.newValidator(100);
        (address validator2, uint256 privKey2, bytes memory publicKey2) = TestUtils.newValidator(101);

        // total collateral in the gateway
        uint256 collateral = 0;
        uint256 stake = 10;
        uint256 validator1Stake = 10 * DEFAULT_MIN_VALIDATOR_STAKE;

        require(!saGetter.isActiveValidator(validator1), "active validator1");
        require(!saGetter.isWaitingValidator(validator1), "waiting validator1");

        require(!saGetter.isActiveValidator(validator2), "active validator2");
        require(!saGetter.isWaitingValidator(validator2), "waiting validator2");

        // ======== Step. Join ======
        // initial validator joins
        vm.deal(validator1, validator1Stake);
        vm.deal(validator2, DEFAULT_MIN_VALIDATOR_STAKE);

        vm.startPrank(validator1);
        saManager.join{value: validator1Stake}(publicKey1);
        collateral = validator1Stake;

        require(gatewayAddress.balance == collateral, "gw balance is incorrect after validator1 joining");

        // collateral confirmed immediately and network boostrapped
        ValidatorInfo memory v = saGetter.getValidator(validator1);
        require(v.totalCollateral == collateral, "total collateral not expected");
        require(v.confirmedCollateral == collateral, "confirmed collateral not equal to collateral");
        require(saGetter.isActiveValidator(validator1), "not active validator 1");
        require(!saGetter.isWaitingValidator(validator1), "waiting validator 1");
        require(!saGetter.isActiveValidator(validator2), "active validator2");
        require(!saGetter.isWaitingValidator(validator2), "waiting validator2");
        TestUtils.ensureBytesEqual(v.metadata, publicKey1);
        require(saGetter.bootstrapped(), "subnet not bootstrapped");
        require(!saGetter.killed(), "subnet killed");
        require(saGetter.genesisValidators().length == 1, "not one validator in genesis");

        (uint64 nextConfigNum, uint64 startConfigNum) = saGetter.getConfigurationNumbers();
        require(nextConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER, "next config num not 1");
        require(startConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER, "start config num not 1");

        vm.stopPrank();

        // subnet bootstrapped and should go through queue
        // second and third validators join
        vm.startPrank(validator2);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKey2);

        // collateral not confirmed yet
        v = saGetter.getValidator(validator2);
        require(gatewayAddress.balance == collateral, "gw balance is incorrect after validator2 joining");
        require(v.totalCollateral == DEFAULT_MIN_VALIDATOR_STAKE, "total collateral not expected");
        require(v.confirmedCollateral == 0, "confirmed collateral not equal to collateral");
        require(saGetter.isActiveValidator(validator1), "not active validator 1");
        require(!saGetter.isWaitingValidator(validator1), "waiting validator 1");
        require(!saGetter.isActiveValidator(validator2), "active validator2");
        require(!saGetter.isWaitingValidator(validator2), "waiting validator2");
        TestUtils.ensureBytesEqual(v.metadata, new bytes(0));

        (nextConfigNum, startConfigNum) = saGetter.getConfigurationNumbers();
        // join will update the metadata, incr by 1
        // join will call deposit, incr by 1
        require(nextConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 2, "next config num not 3");
        require(startConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER, "start config num not 1");
        vm.stopPrank();

        // ======== Step. Confirm join operation ======
        collateral += DEFAULT_MIN_VALIDATOR_STAKE;
        confirmChange(validator1, privKey1);
        require(gatewayAddress.balance == collateral, "gw balance is incorrect after validator2 joining");

        v = saGetter.getValidator(validator2);
        require(v.totalCollateral == DEFAULT_MIN_VALIDATOR_STAKE, "unexpected total collateral after confirm join");
        require(
            v.confirmedCollateral == DEFAULT_MIN_VALIDATOR_STAKE,
            "unexpected confirmed collateral after confirm join"
        );
        require(saGetter.isActiveValidator(validator1), "not active validator1");
        require(!saGetter.isWaitingValidator(validator1), "waiting validator1");
        require(saGetter.isActiveValidator(validator2), "not active validator2");
        require(!saGetter.isWaitingValidator(validator2), "waiting validator2");

        (nextConfigNum, startConfigNum) = saGetter.getConfigurationNumbers();
        require(
            nextConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 2,
            "next config num not 3 after confirm join"
        );
        require(
            startConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 2,
            "start config num not 3 after confirm join"
        );

        // ======== Step. Stake more ======
        vm.startPrank(validator1);
        vm.deal(validator1, stake);

        saManager.stake{value: stake}();

        v = saGetter.getValidator(validator1);
        require(v.totalCollateral == validator1Stake + stake, "unexpected total collateral after stake");
        require(v.confirmedCollateral == validator1Stake, "unexpected confirmed collateral after stake");
        require(gatewayAddress.balance == collateral, "gw balance is incorrect after validator1 stakes more");

        (nextConfigNum, startConfigNum) = saGetter.getConfigurationNumbers();
        require(nextConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 3, "next config num not 4 after stake");
        require(startConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 2, "start config num not 3 after stake");

        vm.stopPrank();

        // ======== Step. Confirm stake operation ======
        collateral += stake;
        confirmChange(validator1, privKey1, validator2, privKey2);
        require(gatewayAddress.balance == collateral, "gw balance is incorrect after confirm stake");

        v = saGetter.getValidator(validator1);
        require(v.totalCollateral == validator1Stake + stake, "unexpected total collateral after confirm stake");
        require(
            v.confirmedCollateral == validator1Stake + stake,
            "unexpected confirmed collateral after confirm stake"
        );

        v = saGetter.getValidator(validator2);
        require(v.totalCollateral == DEFAULT_MIN_VALIDATOR_STAKE, "unexpected total collateral after confirm stake");
        require(
            v.confirmedCollateral == DEFAULT_MIN_VALIDATOR_STAKE,
            "unexpected confirmed collateral after confirm stake"
        );

        (nextConfigNum, startConfigNum) = saGetter.getConfigurationNumbers();
        require(
            nextConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 3,
            "next config num not 4 after confirm stake"
        );
        require(
            startConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 3,
            "start config num not 4 after confirm stake"
        );
        require(saGetter.genesisValidators().length == 1, "genesis validators still 1");

        // ======== Step. Leave ======
        vm.startPrank(validator1);
        saManager.leave();

        v = saGetter.getValidator(validator1);
        require(v.totalCollateral == 0, "total collateral not 0 after confirm leave");
        require(v.confirmedCollateral == validator1Stake + stake, "confirmed collateral incorrect after confirm leave");
        require(gatewayAddress.balance == collateral, "gw balance is incorrect after validator 1 leaving");

        (nextConfigNum, startConfigNum) = saGetter.getConfigurationNumbers();
        require(
            nextConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 4,
            "next config num not 5 after confirm leave"
        );
        require(
            startConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 3,
            "start config num not 4 after confirm leave"
        );
        require(saGetter.isActiveValidator(validator1), "not active validator 1");
        require(saGetter.isActiveValidator(validator2), "not active validator 2");

        vm.stopPrank();

        // ======== Step. Confirm leave ======
        confirmChange(validator1, privKey1);
        collateral -= (validator1Stake + stake);
        require(gatewayAddress.balance == collateral, "gw balance is incorrect after confirming validator 1 leaving");

        v = saGetter.getValidator(validator1);
        require(v.totalCollateral == 0, "total collateral not 0 after confirm leave");
        require(v.confirmedCollateral == 0, "confirmed collateral not 0 after confirm leave");

        (nextConfigNum, startConfigNum) = saGetter.getConfigurationNumbers();
        require(
            nextConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 4,
            "next config num not 5 after confirm leave"
        );
        require(
            startConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER + 4,
            "start config num not 5 after confirm leave"
        );
        require(!saGetter.isActiveValidator(validator1), "active validator 1");
        require(saGetter.isActiveValidator(validator2), "not active validator 2");

        // ======== Step. Claim collateral ======
        uint256 b1 = validator1.balance;
        vm.prank(validator1);
        saRewarder.claim();
        uint256 b2 = validator1.balance;
        require(b2 - b1 == validator1Stake + stake, "collateral not received");
    }

    function testSubnetActorDiamond_Deployment_Works(
        address _ipcGatewayAddr,
        uint256 _minActivationCollateral,
        uint64 _minValidators,
        uint64 _checkPeriod,
        uint8 _majorityPercentage
    ) public {
        vm.assume(_minActivationCollateral > DEFAULT_MIN_VALIDATOR_STAKE);
        vm.assume(_checkPeriod > DEFAULT_CHECKPOINT_PERIOD);
        vm.assume(_majorityPercentage > 51);
        vm.assume(_majorityPercentage <= 100);
        vm.assume(_ipcGatewayAddr != address(0));

        createSubnetActor(
            _ipcGatewayAddr,
            ConsensusType.Fendermint,
            _minActivationCollateral,
            _minValidators,
            _checkPeriod,
            _majorityPercentage
        );

        SubnetID memory parent = saGetter.getParent();
        require(parent.isRoot(), "parent.isRoot()");

        require(saGetter.bottomUpCheckPeriod() == _checkPeriod, "bottomUpCheckPeriod");
    }

    function testSubnetActorDiamond_Deployments_Fail_GatewayCannotBeZero() public {
        SubnetActorManagerFacet saDupMangerFaucet = new SubnetActorManagerFacet();
        SubnetActorGetterFacet saDupGetterFaucet = new SubnetActorGetterFacet();
        SubnetActorPauseFacet saDupPauserFaucet = new SubnetActorPauseFacet();
        SubnetActorRewardFacet saDupRewardFaucet = new SubnetActorRewardFacet();
        SubnetActorCheckpointingFacet saDupCheckpointerFaucet = new SubnetActorCheckpointingFacet();

        SupplySource memory native = SupplySourceHelper.native();

        vm.expectRevert(GatewayCannotBeZero.selector);
        createSubnetActorDiamondWithFaucets(
            SubnetActorDiamond.ConstructorParams({
                parentId: SubnetID(ROOTNET_CHAINID, new address[](0)),
                ipcGatewayAddr: address(0),
                consensus: ConsensusType.Fendermint,
                minActivationCollateral: DEFAULT_MIN_VALIDATOR_STAKE,
                minValidators: DEFAULT_MIN_VALIDATORS,
                bottomUpCheckPeriod: DEFAULT_CHECKPOINT_PERIOD,
                majorityPercentage: DEFAULT_MAJORITY_PERCENTAGE,
                activeValidatorsLimit: 100,
                powerScale: 12,
                permissionMode: PermissionMode.Collateral,
                minCrossMsgFee: DEFAULT_CROSS_MSG_FEE,
                supplySource: native
            }),
            address(saDupGetterFaucet),
            address(saDupMangerFaucet),
            address(saDupPauserFaucet),
            address(saDupRewardFaucet),
            address(saDupCheckpointerFaucet)
        );
    }

    function testSubnetActorDiamond_Join_Fail_NotOwnerOfPublicKey() public {
        address validator = vm.addr(100);

        vm.deal(validator, 1 gwei);
        vm.prank(validator);
        vm.expectRevert(NotOwnerOfPublicKey.selector);

        saManager.join{value: 10}(new bytes(65));
    }

    function testSubnetActorDiamond_Join_Fail_InvalidPublicKeyLength() public {
        address validator = vm.addr(100);

        vm.deal(validator, 1 gwei);
        vm.prank(validator);
        vm.expectRevert(InvalidPublicKeyLength.selector);

        saManager.join{value: 10}(new bytes(64));
    }

    function testSubnetActorDiamond_Join_Fail_ZeroColalteral() public {
        (address validator, bytes memory publicKey) = TestUtils.deriveValidatorAddress(100);

        vm.deal(validator, 1 gwei);
        vm.prank(validator);
        vm.expectRevert(CollateralIsZero.selector);

        saManager.join(publicKey);
    }

    function testSubnetActorDiamond_Bootstrap_Node() public {
        (address validator, uint256 privKey, bytes memory publicKey) = TestUtils.newValidator(100);

        vm.deal(validator, DEFAULT_MIN_VALIDATOR_STAKE);
        vm.prank(validator);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKey);

        // validator adds empty node
        vm.prank(validator);
        vm.expectRevert(EmptyAddress.selector);
        saManager.addBootstrapNode("");

        // validator adds a node
        vm.prank(validator);
        saManager.addBootstrapNode("1.2.3.4");

        // not-validator adds a node
        vm.prank(vm.addr(200));
        vm.expectRevert(abi.encodeWithSelector(NotValidator.selector, vm.addr(200)));
        saManager.addBootstrapNode("3.4.5.6");

        string[] memory nodes = saGetter.getBootstrapNodes();
        require(nodes.length == 1, "it returns one node");
        require(
            keccak256(abi.encodePacked((nodes[0]))) == keccak256(abi.encodePacked(("1.2.3.4"))),
            "it returns correct address"
        );

        vm.prank(validator);
        saManager.leave();
        confirmChange(validator, privKey);

        nodes = saGetter.getBootstrapNodes();
        require(nodes.length == 0, "no nodes");
    }

    function testSubnetActorDiamond_Leave_NotValidator() public {
        (address validator, , ) = TestUtils.newValidator(100);

        // non-empty subnet can't be killed
        vm.prank(validator);
        vm.expectRevert(abi.encodeWithSelector(NotValidator.selector, validator));
        saManager.leave();
    }

    function testSubnetActorDiamond_Leave() public {
        (address validator1, uint256 privKey1, bytes memory publicKey1) = TestUtils.newValidator(100);
        (address validator2, uint256 privKey2, bytes memory publicKey2) = TestUtils.newValidator(101);
        (address validator3, uint256 privKey3, bytes memory publicKey3) = TestUtils.newValidator(102);

        vm.deal(validator1, DEFAULT_MIN_VALIDATOR_STAKE);
        vm.deal(validator2, 3 * DEFAULT_MIN_VALIDATOR_STAKE);
        vm.deal(validator3, DEFAULT_MIN_VALIDATOR_STAKE);

        vm.prank(validator1);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKey1);

        vm.prank(validator2);
        saManager.join{value: 3 * DEFAULT_MIN_VALIDATOR_STAKE}(publicKey2);

        vm.prank(validator3);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKey3);

        confirmChange(validator1, privKey1);

        require(saGetter.isActiveValidator(validator1), "validator 1 is not active");
        require(saGetter.isActiveValidator(validator2), "validator 2 is not active");
        require(saGetter.isActiveValidator(validator3), "validator 3 is not active");

        // non-empty subnet can't be killed
        vm.expectRevert(NotAllValidatorsHaveLeft.selector);
        vm.prank(validator1);
        saManager.kill();

        // validator1 is leaving the subnet
        vm.startPrank(validator1);
        saManager.leave();
        vm.stopPrank();

        confirmChange(validator2, privKey2, validator3, privKey3);

        require(!saGetter.isActiveValidator(validator1), "validator 1 is active");
        require(saGetter.isActiveValidator(validator2), "validator 2 is not active");
        require(saGetter.isActiveValidator(validator3), "validator 3 is not active");
    }

    function testSubnetActorDiamond_Stake() public {
        (address validator, bytes memory publicKey) = TestUtils.deriveValidatorAddress(100);
        vm.deal(validator, 10 gwei);

        vm.prank(validator);
        vm.expectRevert(CollateralIsZero.selector);
        saManager.stake();

        vm.prank(validator);
        vm.expectRevert((abi.encodeWithSelector(MethodNotAllowed.selector, ERR_VALIDATOR_NOT_JOINED)));
        saManager.stake{value: 10}();

        vm.prank(validator);
        saManager.join{value: 3}(publicKey);

        ValidatorInfo memory info = saGetter.getValidator(validator);
        require(info.totalCollateral == 3);
    }

    function testSubnetActorDiamond_crossMsgGetter() public view {
        CrossMsg[] memory msgs = new CrossMsg[](1);
        msgs[0] = CrossMsg({
            message: StorableMsg({
                from: IPCAddress({subnetId: saGetter.getParent(), rawAddress: FvmAddressHelper.from(address(this))}),
                to: IPCAddress({subnetId: saGetter.getParent(), rawAddress: FvmAddressHelper.from(address(this))}),
                value: DEFAULT_CROSS_MSG_FEE + 1,
                nonce: 0,
                method: METHOD_SEND,
                params: new bytes(0),
                fee: DEFAULT_CROSS_MSG_FEE
            }),
            wrapped: false
        });
        require(saGetter.crossMsgsHash(msgs) == keccak256(abi.encode(msgs)));
    }

    function testSubnetActorDiamond_validateActiveQuorumSignatures() public {
        (uint256[] memory keys, address[] memory validators, ) = TestUtils.getThreeValidators(vm);

        bytes[] memory pubKeys = new bytes[](3);
        bytes[] memory signatures = new bytes[](3);

        bytes32 hash = keccak256(abi.encodePacked("test"));

        for (uint256 i = 0; i < 3; i++) {
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[i], hash);
            signatures[i] = abi.encodePacked(r, s, v);
            pubKeys[i] = TestUtils.deriveValidatorPubKeyBytes(keys[i]);
            vm.deal(validators[i], 10 gwei);
            vm.prank(validators[i]);
            saManager.join{value: 10}(pubKeys[i]);
        }

        saCheckpointer.validateActiveQuorumSignatures(validators, hash, signatures);
    }

    function testSubnetActorDiamond_validateActiveQuorumSignatures_InvalidWeightSum() public {
        (uint256[] memory keys, address[] memory validators, ) = TestUtils.getThreeValidators(vm);

        bytes[] memory pubKeys = new bytes[](3);
        bytes[] memory signatures = new bytes[](1);
        address[] memory subValidators = new address[](1);

        bytes32 hash = keccak256(abi.encodePacked("test"));

        for (uint256 i = 0; i < 3; i++) {
            pubKeys[i] = TestUtils.deriveValidatorPubKeyBytes(keys[i]);
            vm.deal(validators[i], 10 gwei);
            vm.prank(validators[i]);
            saManager.join{value: 10}(pubKeys[i]);
        }

        // this should trigger `WeightsSumLessThanThreshold` error since the signature weight will be just 100.
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[0], hash);
        signatures[0] = abi.encodePacked(r, s, v);
        subValidators[0] = validators[0];

        vm.expectRevert(
            abi.encodeWithSelector(
                InvalidSignatureErr.selector,
                MultisignatureChecker.Error.WeightsSumLessThanThreshold
            )
        );
        saCheckpointer.validateActiveQuorumSignatures(subValidators, hash, signatures);
    }

    function testSubnetActorDiamond_validateActiveQuorumSignatures_InvalidSignature() public {
        (uint256[] memory keys, address[] memory validators, ) = TestUtils.getThreeValidators(vm);
        bytes[] memory pubKeys = new bytes[](3);
        bytes[] memory signatures = new bytes[](3);

        bytes32 hash = keccak256(abi.encodePacked("test"));

        uint8 vv = 255;

        for (uint256 i = 0; i < 3; i++) {
            (, bytes32 r, bytes32 s) = vm.sign(keys[i], hash);

            // create incorrect signature using `vv` to trigger `InvalidSignature` error.
            signatures[i] = abi.encodePacked(r, s, vv);

            pubKeys[i] = TestUtils.deriveValidatorPubKeyBytes(keys[i]);
            vm.deal(validators[i], 10 gwei);
            vm.prank(validators[i]);
            saManager.join{value: 10}(pubKeys[i]);
        }

        vm.expectRevert(
            abi.encodeWithSelector(InvalidSignatureErr.selector, MultisignatureChecker.Error.InvalidSignature)
        );
        saCheckpointer.validateActiveQuorumSignatures(validators, hash, signatures);
    }

    function testSubnetActorDiamond_validateActiveQuorumSignatures_EmptySignatures() public {
        (uint256[] memory keys, address[] memory validators, ) = TestUtils.getThreeValidators(vm);
        bytes[] memory pubKeys = new bytes[](3);
        bytes[] memory signatures = new bytes[](0);

        bytes32 hash = keccak256(abi.encodePacked("test"));

        for (uint256 i = 0; i < 3; i++) {
            pubKeys[i] = TestUtils.deriveValidatorPubKeyBytes(keys[i]);
            vm.deal(validators[i], 10 gwei);
            vm.prank(validators[i]);
            saManager.join{value: 10}(pubKeys[i]);
        }

        require(signatures.length == 0, "signatures are not empty");
        vm.expectRevert(
            abi.encodeWithSelector(InvalidSignatureErr.selector, MultisignatureChecker.Error.EmptySignatures)
        );
        saCheckpointer.validateActiveQuorumSignatures(validators, hash, signatures);
    }

    function testSubnetActorDiamond_validateActiveQuorumSignatures_InvalidArrayLength() public {
        (uint256[] memory keys, address[] memory validators, ) = TestUtils.getThreeValidators(vm);
        bytes[] memory pubKeys = new bytes[](3);
        bytes[] memory signatures = new bytes[](1);

        bytes32 hash = keccak256(abi.encodePacked("test"));

        for (uint256 i = 0; i < 3; i++) {
            pubKeys[i] = TestUtils.deriveValidatorPubKeyBytes(keys[i]);
            vm.deal(validators[i], 10 gwei);
            vm.prank(validators[i]);
            saManager.join{value: 10}(pubKeys[i]);
        }

        require(signatures.length == 1, "signatures are not empty");
        vm.expectRevert(
            abi.encodeWithSelector(InvalidSignatureErr.selector, MultisignatureChecker.Error.InvalidArrayLength)
        );
        saCheckpointer.validateActiveQuorumSignatures(validators, hash, signatures);
    }

    function testSubnetActorDiamond_validateActiveQuorumSignatures_InvalidSignatory() public {
        (uint256[] memory keys, address[] memory validators, ) = TestUtils.getThreeValidators(vm);
        bytes[] memory pubKeys = new bytes[](3);
        bytes[] memory signatures = new bytes[](3);

        bytes32 hash = keccak256(abi.encodePacked("test"));
        bytes32 hash0 = keccak256(abi.encodePacked("test1"));

        for (uint256 i = 0; i < 3; i++) {
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[i], hash);

            // create incorrect signature using `vv`
            signatures[i] = abi.encodePacked(r, s, v);

            pubKeys[i] = TestUtils.deriveValidatorPubKeyBytes(keys[i]);
            vm.deal(validators[i], 10 gwei);
            vm.prank(validators[i]);
            saManager.join{value: 10}(pubKeys[i]);
        }

        // swap validators to trigger `InvalidSignatory` error;
        address a;
        a = validators[0];
        validators[0] = validators[1];
        validators[1] = a;

        vm.expectRevert(
            abi.encodeWithSelector(InvalidSignatureErr.selector, MultisignatureChecker.Error.InvalidSignatory)
        );
        saCheckpointer.validateActiveQuorumSignatures(validators, hash0, signatures);
    }

    function testSubnetActorDiamond_submitCheckpoint_basic() public {
        (uint256[] memory keys, address[] memory validators, ) = TestUtils.getThreeValidators(vm);
        bytes[] memory pubKeys = new bytes[](3);
        bytes[] memory signatures = new bytes[](3);

        for (uint256 i = 0; i < 3; i++) {
            vm.deal(validators[i], 10 gwei);
            pubKeys[i] = TestUtils.deriveValidatorPubKeyBytes(keys[i]);
            vm.prank(validators[i]);
            saManager.join{value: 10}(pubKeys[i]);
        }

        CrossMsg memory crossMsg = CrossMsg({
            message: StorableMsg({
                from: IPCAddress({
                    subnetId: saGetter.getParent(),
                    rawAddress: FvmAddressHelper.from(address(saDiamond))
                }),
                to: IPCAddress({subnetId: saGetter.getParent(), rawAddress: FvmAddressHelper.from(address(saDiamond))}),
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

        BottomUpCheckpoint memory checkpoint = BottomUpCheckpoint({
            subnetID: saGetter.getParent().createSubnetId(address(saDiamond)),
            blockHeight: saGetter.bottomUpCheckPeriod(),
            blockHash: keccak256("block1"),
            nextConfigurationNumber: 0
        });

        BottomUpCheckpoint memory checkpointWithIncorrectHeight = BottomUpCheckpoint({
            subnetID: saGetter.getParent(),
            blockHeight: 1,
            blockHash: keccak256("block1"),
            nextConfigurationNumber: 0
        });

        vm.deal(address(saDiamond), 100 ether);
        vm.prank(address(saDiamond));
        gwManager.register{value: DEFAULT_MIN_VALIDATOR_STAKE + 3 * DEFAULT_CROSS_MSG_FEE}(3 * DEFAULT_CROSS_MSG_FEE);

        bytes32 hash = keccak256(abi.encode(checkpoint));

        for (uint256 i = 0; i < 3; i++) {
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[i], hash);
            signatures[i] = abi.encodePacked(r, s, v);
        }

        vm.expectRevert(InvalidCheckpointEpoch.selector);
        vm.prank(validators[0]);
        saCheckpointer.submitCheckpoint(checkpointWithIncorrectHeight, validators, signatures);

        vm.expectCall(gatewayAddress, abi.encodeCall(IGateway.commitCheckpoint, (checkpoint)), 1);
        vm.prank(validators[0]);
        saCheckpointer.submitCheckpoint(checkpoint, validators, signatures);

        require(saGetter.hasSubmittedInLastBottomUpCheckpointHeight(validators[0]), "validator rewarded");
        require(
            saGetter.lastBottomUpCheckpointHeight() == saGetter.bottomUpCheckPeriod(),
            " checkpoint height correct"
        );

        vm.prank(validators[0]);
        saCheckpointer.submitCheckpoint(checkpoint, validators, signatures);
        require(saGetter.hasSubmittedInLastBottomUpCheckpointHeight(validators[0]), "validator rewarded");
        require(
            saGetter.lastBottomUpCheckpointHeight() == saGetter.bottomUpCheckPeriod(),
            " checkpoint height correct"
        );

        (bool exists, BottomUpCheckpoint memory recvCheckpoint) = saGetter.bottomUpCheckpointAtEpoch(
            saGetter.bottomUpCheckPeriod()
        );
        require(exists, "checkpoint does not exist");
        require(hash == keccak256(abi.encode(recvCheckpoint)), "checkpoint hashes are not the same");

        bytes32 recvHash;
        (exists, recvHash) = saGetter.bottomUpCheckpointHashAtEpoch(saGetter.bottomUpCheckPeriod());
        require(exists, "checkpoint does not exist");
        require(hash == recvHash, "hashes are not the same");
    }

    function testSubnetActorDiamond_submitCheckpointWithReward() public {
        (uint256[] memory keys, address[] memory validators, ) = TestUtils.getThreeValidators(vm);
        bytes[] memory pubKeys = new bytes[](3);
        bytes[] memory signatures = new bytes[](3);

        for (uint256 i = 0; i < 3; i++) {
            vm.deal(validators[i], 10 gwei);
            pubKeys[i] = TestUtils.deriveValidatorPubKeyBytes(keys[i]);
            vm.prank(validators[i]);
            saManager.join{value: 10}(pubKeys[i]);
        }

        // send the first checkpoint
        CrossMsg memory crossMsg = CrossMsg({
            message: StorableMsg({
                from: IPCAddress({
                    subnetId: saGetter.getParent(),
                    rawAddress: FvmAddressHelper.from(address(saDiamond))
                }),
                to: IPCAddress({subnetId: saGetter.getParent(), rawAddress: FvmAddressHelper.from(address(saDiamond))}),
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

        BottomUpCheckpoint memory checkpoint = BottomUpCheckpoint({
            subnetID: saGetter.getParent().createSubnetId(address(saDiamond)),
            blockHeight: saGetter.bottomUpCheckPeriod(),
            blockHash: keccak256("block1"),
            nextConfigurationNumber: 0
        });

        vm.deal(address(saDiamond), 100 ether);
        vm.prank(address(saDiamond));
        gwManager.register{value: DEFAULT_MIN_VALIDATOR_STAKE + 6 * DEFAULT_CROSS_MSG_FEE}(6 * DEFAULT_CROSS_MSG_FEE);

        bytes32 hash = keccak256(abi.encode(checkpoint));

        for (uint256 i = 0; i < 3; i++) {
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[i], hash);
            signatures[i] = abi.encodePacked(r, s, v);
        }

        vm.expectCall(gatewayAddress, abi.encodeCall(IGateway.commitCheckpoint, (checkpoint)), 1);
        vm.prank(validators[0]);
        saCheckpointer.submitCheckpoint(checkpoint, validators, signatures);

        require(saGetter.hasSubmittedInLastBottomUpCheckpointHeight(validators[0]), "validator rewarded");
        require(
            saGetter.lastBottomUpCheckpointHeight() == saGetter.bottomUpCheckPeriod(),
            " checkpoint height correct"
        );

        require(saGetter.getRelayerReward(validators[0]) == 0, "there is a reward");

        // send the second checkpoint
        crossMsg = CrossMsg({
            message: StorableMsg({
                from: IPCAddress({
                    subnetId: saGetter.getParent(),
                    rawAddress: FvmAddressHelper.from(address(saDiamond))
                }),
                to: IPCAddress({subnetId: saGetter.getParent(), rawAddress: FvmAddressHelper.from(address(saDiamond))}),
                value: DEFAULT_CROSS_MSG_FEE + 1,
                nonce: 1,
                method: METHOD_SEND,
                params: new bytes(0),
                fee: DEFAULT_CROSS_MSG_FEE
            }),
            wrapped: false
        });
        msgs[0] = crossMsg;

        checkpoint = BottomUpCheckpoint({
            subnetID: saGetter.getParent().createSubnetId(address(saDiamond)),
            blockHeight: 2 * saGetter.bottomUpCheckPeriod(),
            blockHash: keccak256("block2"),
            nextConfigurationNumber: 0
        });

        hash = keccak256(abi.encode(checkpoint));

        for (uint256 i = 0; i < 3; i++) {
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[i], hash);
            signatures[i] = abi.encodePacked(r, s, v);
        }

        vm.prank(validators[0]);
        saCheckpointer.submitCheckpoint(checkpoint, validators, signatures);

        require(saGetter.getRelayerReward(validators[1]) == 0, "unexpected reward");
        require(saGetter.getRelayerReward(validators[2]) == 0, "unexpected reward");
        uint256 validator0Reward = saGetter.getRelayerReward(validators[0]);
        if (FEATURE_CHECKPOINT_RELAYER_REWARDS) {
            require(validator0Reward == DEFAULT_CROSS_MSG_FEE, "there is no reward for validator");

            uint256 b1 = validators[0].balance;
            // disable the claim of rewards if the fee is zero
            if (DEFAULT_CROSS_MSG_FEE != 0) {
                vm.startPrank(validators[0]);
                saRewarder.claimRewardForRelayer();
                uint256 b2 = validators[0].balance;
                require(b2 - b1 == validator0Reward, "reward received");
            }
        }
    }

    function testSubnetActorDiamond_submitBottomUpMsgBatch_basic() public {
        (uint256[] memory keys, address[] memory validators, ) = TestUtils.getThreeValidators(vm);
        bytes[] memory pubKeys = new bytes[](3);
        bytes[] memory signatures = new bytes[](3);
        uint256 amount = 1;

        for (uint256 i = 0; i < 3; i++) {
            vm.deal(validators[i], 10 gwei);
            pubKeys[i] = TestUtils.deriveValidatorPubKeyBytes(keys[i]);
            vm.prank(validators[i]);
            saManager.join{value: 10}(pubKeys[i]);
        }

        CrossMsg memory crossMsg = CrossMsg({
            message: StorableMsg({
                from: IPCAddress({
                    subnetId: saGetter.getParent(),
                    rawAddress: FvmAddressHelper.from(address(saDiamond))
                }),
                to: IPCAddress({subnetId: saGetter.getParent(), rawAddress: FvmAddressHelper.from(address(saDiamond))}),
                value: DEFAULT_CROSS_MSG_FEE + amount,
                nonce: 0,
                method: METHOD_SEND,
                params: new bytes(0),
                fee: DEFAULT_CROSS_MSG_FEE
            }),
            wrapped: false
        });
        CrossMsg[] memory msgs = new CrossMsg[](1);
        msgs[0] = crossMsg;

        BottomUpMsgBatch memory batch = BottomUpMsgBatch({
            subnetID: saGetter.getParent().createSubnetId(address(saDiamond)),
            blockHeight: saGetter.bottomUpMsgBatchPeriod(),
            msgs: msgs
        });

        BottomUpMsgBatch memory batchIncorrectHeight = BottomUpMsgBatch({
            subnetID: saGetter.getParent().createSubnetId(address(saDiamond)),
            blockHeight: 1,
            msgs: msgs
        });

        vm.deal(address(saDiamond), 100 ether);
        vm.prank(address(saDiamond));
        // register with circulating supply
        gwManager.register{value: 3 * DEFAULT_MIN_VALIDATOR_STAKE}(3 * amount + 3 * DEFAULT_CROSS_MSG_FEE);

        bytes32 hash = keccak256(abi.encode(batch));

        for (uint256 i = 0; i < 3; i++) {
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[i], hash);
            signatures[i] = abi.encodePacked(r, s, v);
        }

        vm.expectRevert(InvalidBatchEpoch.selector);
        vm.prank(validators[0]);
        saCheckpointer.submitBottomUpMsgBatch(batchIncorrectHeight, validators, signatures);

        vm.prank(validators[0]);
        batchIncorrectHeight.msgs = new CrossMsg[](0);
        batchIncorrectHeight.blockHeight = saGetter.bottomUpMsgBatchPeriod();
        vm.expectRevert(BatchWithNoMessages.selector);
        saCheckpointer.submitBottomUpMsgBatch(batchIncorrectHeight, validators, signatures);

        vm.expectCall(gatewayAddress, abi.encodeCall(IGateway.execBottomUpMsgBatch, (batch)), 1);
        vm.prank(validators[0]);
        saCheckpointer.submitBottomUpMsgBatch(batch, validators, signatures);

        require(saGetter.hasSubmittedInLastBottomUpMsgBatchHeight(validators[0]), "validator rewarded");
        require(saGetter.lastBottomUpMsgBatchHeight() == saGetter.bottomUpMsgBatchPeriod(), " batch height correct");

        vm.prank(validators[1]);
        saCheckpointer.submitBottomUpMsgBatch(batch, validators, signatures);
        require(saGetter.hasSubmittedInLastBottomUpMsgBatchHeight(validators[0]), "validator rewarded");
        require(saGetter.lastBottomUpMsgBatchHeight() == saGetter.bottomUpMsgBatchPeriod(), " batch height correct");
    }

    function testSubnetActorDiamond_DiamondCut() public {
        // add method getNum to subnet actor diamond and assert it can be correctly called
        // replace method getNum and assert it was correctly updated
        // delete method getNum and assert it no longer is callable
        // assert that diamondCut cannot be called by non-owner

        NumberContractFacetSeven ncFacetA = new NumberContractFacetSeven();
        NumberContractFacetEight ncFacetB = new NumberContractFacetEight();

        DiamondCutFacet saDiamondCutter = DiamondCutFacet(address(saDiamond));
        IDiamond.FacetCut[] memory saDiamondCut = new IDiamond.FacetCut[](1);
        bytes4[] memory ncGetterSelectors = SelectorLibrary.resolveSelectors("NumberContractFacetSeven");

        saDiamondCut[0] = (
            IDiamond.FacetCut({
                facetAddress: address(ncFacetA),
                action: IDiamond.FacetCutAction.Add,
                functionSelectors: ncGetterSelectors
            })
        );
        //test that other user cannot call diamondcut to add function
        vm.prank(0x1234567890123456789012345678901234567890);
        vm.expectRevert(LibDiamond.NotOwner.selector);
        saDiamondCutter.diamondCut(saDiamondCut, address(0), new bytes(0));

        saDiamondCutter.diamondCut(saDiamondCut, address(0), new bytes(0));

        NumberContractFacetSeven saNumberContract = NumberContractFacetSeven(address(saDiamond));
        assert(saNumberContract.getNum() == 7);

        ncGetterSelectors = SelectorLibrary.resolveSelectors("NumberContractFacetEight");
        saDiamondCut[0] = (
            IDiamond.FacetCut({
                facetAddress: address(ncFacetB),
                action: IDiamond.FacetCutAction.Replace,
                functionSelectors: ncGetterSelectors
            })
        );

        //test that other user cannot call diamondcut to replace function
        vm.prank(0x1234567890123456789012345678901234567890);
        vm.expectRevert(LibDiamond.NotOwner.selector);
        saDiamondCutter.diamondCut(saDiamondCut, address(0), new bytes(0));

        saDiamondCutter.diamondCut(saDiamondCut, address(0), new bytes(0));

        assert(saNumberContract.getNum() == 8);

        //remove facet for getNum
        saDiamondCut[0] = (
            IDiamond.FacetCut({
                facetAddress: 0x0000000000000000000000000000000000000000,
                action: IDiamond.FacetCutAction.Remove,
                functionSelectors: ncGetterSelectors
            })
        );

        //test that other user cannot call diamondcut to remove function
        vm.prank(0x1234567890123456789012345678901234567890);
        vm.expectRevert(LibDiamond.NotOwner.selector);
        saDiamondCutter.diamondCut(saDiamondCut, address(0), new bytes(0));

        saDiamondCutter.diamondCut(saDiamondCut, address(0), new bytes(0));

        //assert that calling getNum fails
        vm.expectRevert(abi.encodePacked(FunctionNotFound.selector, ncGetterSelectors));
        saNumberContract.getNum();
    }

    function testSubnetActorDiamond_Unstake() public {
        (address validator, bytes memory publicKey) = TestUtils.deriveValidatorAddress(100);

        vm.expectRevert(CannotReleaseZero.selector);
        vm.prank(validator);
        saManager.unstake(0);

        vm.expectRevert(abi.encodeWithSelector(NotValidator.selector, validator));
        vm.prank(validator);
        saManager.unstake(10);

        vm.deal(validator, DEFAULT_MIN_VALIDATOR_STAKE);
        vm.prank(validator);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKey);
        require(
            saGetter.getValidator(validator).totalCollateral == DEFAULT_MIN_VALIDATOR_STAKE,
            "initial collateral correct"
        );

        vm.expectRevert(NotEnoughCollateral.selector);
        vm.prank(validator);
        saManager.unstake(DEFAULT_MIN_VALIDATOR_STAKE + 100);

        vm.expectRevert(NotEnoughCollateral.selector);
        vm.prank(validator);
        saManager.unstake(DEFAULT_MIN_VALIDATOR_STAKE);

        vm.prank(validator);
        saManager.unstake(5);
        require(
            saGetter.getValidator(validator).totalCollateral == DEFAULT_MIN_VALIDATOR_STAKE - 5,
            "collateral correct after unstaking"
        );
    }

    function testSubnetActorDiamond_PreFundRelease_works() public {
        (address validator1, bytes memory publicKey1) = TestUtils.deriveValidatorAddress(100);
        address preFunder = address(102);
        address preReleaser = address(103);

        // total collateral in the gateway
        uint256 collateral = 0;
        uint256 fundAmount = 100;

        require(!saGetter.isActiveValidator(validator1), "active validator1");
        require(!saGetter.isWaitingValidator(validator1), "waiting validator1");

        // ======== Step. Join ======

        // pre-fund and pre-release from same address
        vm.startPrank(preReleaser);
        vm.deal(preReleaser, 2 * fundAmount);
        saManager.preFund{value: 2 * fundAmount}();
        require(saGetter.genesisCircSupply() == 2 * fundAmount, "genesis circ supply not correct");
        saManager.preRelease(fundAmount);
        require(saGetter.genesisCircSupply() == fundAmount, "genesis circ supply not correct");
        (address[] memory genesisAddrs, ) = saGetter.genesisBalances();
        require(genesisAddrs.length == 1, "not one genesis addresses");
        // cannot release more than the initial balance of the address
        vm.expectRevert(NotEnoughBalance.selector);
        saManager.preRelease(2 * fundAmount);
        // release all
        saManager.preRelease(fundAmount);
        (genesisAddrs, ) = saGetter.genesisBalances();
        require(saGetter.genesisCircSupply() == 0, "genesis circ supply not correct");
        require(genesisAddrs.length == 0, "not zero genesis addresses");
        vm.stopPrank();

        // pre-fund from validator and from pre-funder
        vm.startPrank(validator1);
        vm.deal(validator1, fundAmount);
        saManager.preFund{value: fundAmount}();
        vm.stopPrank();

        vm.startPrank(preFunder);
        vm.deal(preFunder, fundAmount);
        saManager.preFund{value: fundAmount}();
        vm.stopPrank();

        // initial validator joins
        vm.deal(validator1, DEFAULT_MIN_VALIDATOR_STAKE);

        vm.startPrank(validator1);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKey1);
        vm.stopPrank();
        collateral = DEFAULT_MIN_VALIDATOR_STAKE;

        require(
            gatewayAddress.balance == collateral + 2 * fundAmount,
            "gw balance is incorrect after validator1 joining"
        );

        require(saGetter.genesisCircSupply() == 2 * fundAmount, "genesis circ supply not correct");
        (genesisAddrs, ) = saGetter.genesisBalances();
        require(genesisAddrs.length == 2, "not two genesis addresses");

        // collateral confirmed immediately and network boostrapped
        ValidatorInfo memory v = saGetter.getValidator(validator1);
        require(v.totalCollateral == DEFAULT_MIN_VALIDATOR_STAKE, "total collateral not expected");
        require(v.confirmedCollateral == DEFAULT_MIN_VALIDATOR_STAKE, "confirmed collateral not equal to collateral");
        require(saGetter.isActiveValidator(validator1), "not active validator 1");
        require(!saGetter.isWaitingValidator(validator1), "waiting validator 1");
        TestUtils.ensureBytesEqual(v.metadata, publicKey1);
        require(saGetter.bootstrapped(), "subnet not bootstrapped");
        require(!saGetter.killed(), "subnet killed");
        require(saGetter.genesisValidators().length == 1, "not one validator in genesis");

        (uint64 nextConfigNum, uint64 startConfigNum) = saGetter.getConfigurationNumbers();
        require(nextConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER, "next config num not 1");
        require(startConfigNum == LibStaking.INITIAL_CONFIGURATION_NUMBER, "start config num not 1");

        // pre-fund not allowed with bootstrapped subnet
        vm.startPrank(preFunder);
        vm.expectRevert(SubnetAlreadyBootstrapped.selector);
        vm.deal(preFunder, fundAmount);
        saManager.preFund{value: fundAmount}();
        vm.stopPrank();
    }

    function testSubnetActorDiamond_PreFundAndLeave_works() public {
        (address validator1, bytes memory publicKey1) = TestUtils.deriveValidatorAddress(100);

        // total collateral in the gateway
        uint256 collateral = DEFAULT_MIN_VALIDATOR_STAKE - 100;
        uint256 fundAmount = 100;

        // pre-fund from validator
        vm.startPrank(validator1);
        vm.deal(validator1, fundAmount);
        saManager.preFund{value: fundAmount}();
        vm.stopPrank();

        // initial validator joins but doesn't bootstrap the subnet
        vm.deal(validator1, collateral);
        vm.startPrank(validator1);
        saManager.join{value: collateral}(publicKey1);
        require(
            address(saDiamond).balance == collateral + fundAmount,
            "subnet balance is incorrect after validator1 joining"
        );
        require(saGetter.genesisCircSupply() == fundAmount, "genesis circ supply not correct");
        (address[] memory genesisAddrs, ) = saGetter.genesisBalances();
        require(genesisAddrs.length == 1, "not one genesis addresses");

        // Leave should return the collateral and the initial balance
        saManager.leave();
        require(address(saDiamond).balance == 0, "subnet balance is incorrect after validator1 leaving");
        require(saGetter.genesisCircSupply() == 0, "genesis circ supply not zero");
        (genesisAddrs, ) = saGetter.genesisBalances();
        require(genesisAddrs.length == 0, "not zero genesis addresses");
        vm.stopPrank();

        // initial validator joins to bootstrap the subnet
        vm.deal(validator1, DEFAULT_MIN_VALIDATOR_STAKE);

        vm.startPrank(validator1);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKey1);
        vm.stopPrank();

        // pre-release not allowed with bootstrapped subnet
        vm.startPrank(validator1);
        vm.expectRevert(SubnetAlreadyBootstrapped.selector);
        saManager.preRelease(fundAmount);
        vm.stopPrank();
    }

    function testSubnetActorDiamond_MultipleJoins_Works_GetValidators() public {
        uint256 n = 10;

        (address[] memory validators, uint256[] memory privKeys, bytes[] memory publicKeys) = TestUtils.newValidators(
            n
        );

        for (uint i = 0; i < n; i++) {
            vm.deal(validators[i], 100 * DEFAULT_MIN_VALIDATOR_STAKE);
        }

        vm.prank(validators[0]);
        saManager.join{value: 100 * DEFAULT_MIN_VALIDATOR_STAKE}(publicKeys[0]);

        for (uint i = 1; i < n; i++) {
            vm.prank(validators[i]);
            saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKeys[i]);
        }

        confirmChange(validators[0], privKeys[0]);

        for (uint i = 0; i < n; i++) {
            require(saGetter.isActiveValidator(validators[i]), "not active validator");
        }
    }

    function testSubnetActorDiamond_Join_Works_WithMinimalStake() public {
        uint256 n = 10;

        (address[] memory validators, uint256[] memory privKeys, bytes[] memory publicKeys) = TestUtils.newValidators(
            n
        );

        vm.deal(validators[0], 100 * DEFAULT_MIN_VALIDATOR_STAKE);
        for (uint i = 1; i < n; i++) {
            vm.deal(validators[i], 1);
        }

        vm.prank(validators[0]);
        saManager.join{value: 100 * DEFAULT_MIN_VALIDATOR_STAKE}(publicKeys[0]);

        for (uint i = 1; i < n; i++) {
            vm.prank(validators[i]);
            saManager.join{value: 1}(publicKeys[i]);
        }

        confirmChange(validators[0], privKeys[0]);

        for (uint i = 0; i < n; i++) {
            require(saGetter.isActiveValidator(validators[i]), "not active validator");
        }
    }

    function testSubnetActorDiamond_NotBootstrapped_LessThanActivation() public {
        uint256 n = 10;

        (address[] memory validators, , bytes[] memory publicKeys) = TestUtils.newValidators(n);

        for (uint i = 0; i < n; i++) {
            vm.deal(validators[i], 1);
            vm.prank(validators[i]);
            saManager.join{value: 1}(publicKeys[i]);
        }

        require(!saGetter.bootstrapped());
    }

    function test_second_validator_can_join() public {
        (address validatorAddress1, uint256 privKey1, bytes memory publicKey1) = TestUtils.newValidator(101);
        (address validatorAddress2, , bytes memory publicKey2) = TestUtils.newValidator(102);

        join(validatorAddress1, publicKey1);

        require(saGetter.bootstrapped(), "subnet not bootstrapped");
        require(saGetter.isActiveValidator(validatorAddress1), "validator 1 is not active");
        require(!saGetter.isActiveValidator(validatorAddress2), "validator 2 is active");

        join(validatorAddress2, publicKey2);
        confirmChange(validatorAddress1, privKey1);
        require(saGetter.isActiveValidator(validatorAddress2), "validator 2 is not active");
    }

    function callback() public view {
        // console.log("callback called");
    }

    function testSubnetActorDiamond_StaticValidation_cannotJoin() public {
        gatewayAddress = address(gatewayDiamond);

        createSubnetActor(
            gatewayAddress,
            ConsensusType.Fendermint,
            DEFAULT_MIN_VALIDATOR_STAKE,
            DEFAULT_MIN_VALIDATORS,
            DEFAULT_CHECKPOINT_PERIOD,
            DEFAULT_MAJORITY_PERCENTAGE,
            PermissionMode.Static,
            2
        );

        (address validator1, bytes memory publicKey1) = TestUtils.deriveValidatorAddress(100);
        vm.deal(validator1, DEFAULT_MIN_VALIDATOR_STAKE * 2);
        vm.prank(validator1);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE / 2}(publicKey1);

        (address validator2, bytes memory publicKey2) = TestUtils.deriveValidatorAddress(101);
        vm.deal(validator2, DEFAULT_MIN_VALIDATOR_STAKE * 2);
        vm.prank(validator2);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE / 2}(publicKey2);

        require(saGetter.isActiveValidator(validator1), "not active validator 1");
        require(saGetter.isActiveValidator(validator2), "not active validator 2");

        // cannot join after bootstrap

        vm.expectRevert(abi.encodeWithSelector(MethodNotAllowed.selector, ERR_PERMISSIONED_AND_BOOTSTRAPPED));
        vm.prank(validator1);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKey1);

        vm.expectRevert(abi.encodeWithSelector(MethodNotAllowed.selector, ERR_PERMISSIONED_AND_BOOTSTRAPPED));
        (address[] memory validators, , bytes[] memory publicKeys) = TestUtils.newValidators(3);
        uint256[] memory powers = new uint256[](3);
        powers[0] = 10000;
        powers[1] = 20000;
        powers[2] = 5000; // we only have 2 active validators, validator 2 does not have enough power
        saManager.setFederatedPower(validators, publicKeys, powers);
    }

    function testSubnetActorDiamond_registration_policy() public {
        (address validator1, bytes memory publicKey1) = TestUtils.deriveValidatorAddress(100);
        vm.deal(validator1, DEFAULT_MIN_VALIDATOR_STAKE);

        vm.expectRevert(abi.encodeWithSelector(MethodNotAllowed.selector, ERR_VALIDATOR_NOT_JOINED));
        vm.prank(validator1);
        saManager.stake{value: DEFAULT_MIN_VALIDATOR_STAKE / 2}();

        vm.prank(validator1);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE / 2}(publicKey1);

        require(saGetter.isActiveValidator(validator1), "active validator 1");
        require(!saGetter.bootstrapped(), "subnet bootstrapped");

        vm.expectRevert(abi.encodeWithSelector(MethodNotAllowed.selector, ERR_VALIDATOR_JOINED));
        vm.prank(validator1);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE / 2}(publicKey1);

        vm.prank(validator1);
        saManager.stake{value: DEFAULT_MIN_VALIDATOR_STAKE / 2}();

        require(saGetter.isActiveValidator(validator1), "active validator 1");

        (address validator2, bytes memory publicKey2) = TestUtils.deriveValidatorAddress(101);
        vm.deal(validator2, DEFAULT_MIN_VALIDATOR_STAKE * 2);
        vm.prank(validator2);
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE / 2}(publicKey2);

        require(saGetter.isActiveValidator(validator1), "not active validator 1");
        require(saGetter.bootstrapped(), "subnet not bootstrapped");
    }

    function testSubnetActorDiamond_FederatedValidation_bootstrapWorks() public {
        gatewayAddress = address(gatewayDiamond);

        createSubnetActor(
            gatewayAddress,
            ConsensusType.Fendermint,
            DEFAULT_MIN_VALIDATOR_STAKE,
            DEFAULT_MIN_VALIDATORS,
            DEFAULT_CHECKPOINT_PERIOD,
            DEFAULT_MAJORITY_PERCENTAGE,
            PermissionMode.Federated,
            2
        );

        (address[] memory validators, , bytes[] memory publicKeys) = TestUtils.newValidators(2);

        uint256[] memory powers = new uint256[](2);
        powers[0] = 10000;
        powers[1] = 20000;

        saManager.setFederatedPower(validators, publicKeys, powers);
        require(saGetter.isActiveValidator(validators[0]), "not active validator 1");
        require(saGetter.isActiveValidator(validators[1]), "not active validator 2");
    }

    function testSubnetActorDiamond_FederatedValidation_bootstrapNotOwnerOfPublicKeys() public {
        gatewayAddress = address(gatewayDiamond);

        createSubnetActor(
            gatewayAddress,
            ConsensusType.Fendermint,
            DEFAULT_MIN_VALIDATOR_STAKE,
            DEFAULT_MIN_VALIDATORS,
            DEFAULT_CHECKPOINT_PERIOD,
            DEFAULT_MAJORITY_PERCENTAGE,
            PermissionMode.Federated,
            2
        );

        (address[] memory validators, , bytes[] memory publicKeys) = TestUtils.newValidators(2);
        publicKeys[1] = publicKeys[0];

        uint256[] memory powers = new uint256[](2);
        powers[0] = 10000;
        powers[1] = 20000;

        vm.expectRevert(abi.encodeWithSelector(NotOwnerOfPublicKey.selector));
        saManager.setFederatedPower(validators, publicKeys, powers);
    }

    function testSubnetActorDiamond_FederatedValidation_bootstrapNotEnoughValidators() public {
        gatewayAddress = address(gatewayDiamond);

        createSubnetActor(
            gatewayAddress,
            ConsensusType.Fendermint,
            DEFAULT_MIN_VALIDATOR_STAKE,
            DEFAULT_MIN_VALIDATORS,
            DEFAULT_CHECKPOINT_PERIOD,
            DEFAULT_MAJORITY_PERCENTAGE,
            PermissionMode.Federated,
            2
        );

        (address[] memory validators, , bytes[] memory publicKeys) = TestUtils.newValidators(1);

        uint256[] memory powers = new uint256[](1);
        powers[0] = 10000;

        vm.expectRevert(abi.encodeWithSelector(NotEnoughGenesisValidators.selector));
        saManager.setFederatedPower(validators, publicKeys, powers);
    }

    function testSubnetActorDiamond_FederatedValidation_bootstrapDuplicates() public {
        gatewayAddress = address(gatewayDiamond);

        createSubnetActor(
            gatewayAddress,
            ConsensusType.Fendermint,
            DEFAULT_MIN_VALIDATOR_STAKE,
            DEFAULT_MIN_VALIDATORS,
            DEFAULT_CHECKPOINT_PERIOD,
            DEFAULT_MAJORITY_PERCENTAGE,
            PermissionMode.Federated,
            2
        );

        (address[] memory validators, , bytes[] memory publicKeys) = TestUtils.newValidators(2);
        validators[1] = validators[0];
        publicKeys[1] = publicKeys[0];

        uint256[] memory powers = new uint256[](2);
        powers[0] = 10000;
        powers[1] = 20000;

        vm.expectRevert(abi.encodeWithSelector(DuplicatedGenesisValidator.selector));
        saManager.setFederatedPower(validators, publicKeys, powers);
    }

    function testSubnetActorDiamond_FederatedValidation_cannotJoin() public {
        gatewayAddress = address(gatewayDiamond);

        createSubnetActor(
            gatewayAddress,
            ConsensusType.Fendermint,
            DEFAULT_MIN_VALIDATOR_STAKE,
            DEFAULT_MIN_VALIDATORS,
            DEFAULT_CHECKPOINT_PERIOD,
            DEFAULT_MAJORITY_PERCENTAGE,
            PermissionMode.Federated,
            2
        );

        (address[] memory validators, , bytes[] memory publicKeys) = TestUtils.newValidators(2);
        uint256[] memory powers = new uint256[](2);
        powers[0] = 10000;
        powers[1] = 20000;

        saManager.setFederatedPower(validators, publicKeys, powers);

        vm.deal(validators[0], DEFAULT_MIN_VALIDATOR_STAKE * 2);
        vm.startPrank(validators[0]);
        vm.expectRevert(abi.encodeWithSelector(MethodNotAllowed.selector, ERR_PERMISSIONED_AND_BOOTSTRAPPED));
        saManager.join{value: DEFAULT_MIN_VALIDATOR_STAKE}(publicKeys[0]);
    }

    function testSubnetActorDiamond_FederatedValidation_works() public {
        gatewayAddress = address(gatewayDiamond);

        createSubnetActor(
            gatewayAddress,
            ConsensusType.Fendermint,
            DEFAULT_MIN_VALIDATOR_STAKE,
            DEFAULT_MIN_VALIDATORS,
            DEFAULT_CHECKPOINT_PERIOD,
            DEFAULT_MAJORITY_PERCENTAGE,
            PermissionMode.Federated,
            2
        );

        (address[] memory validators, uint256[] memory privKeys, bytes[] memory publicKeys) = TestUtils.newValidators(
            3
        );
        uint256[] memory powers = new uint256[](3);
        powers[0] = 10000;
        powers[1] = 20000;
        powers[2] = 5000; // we only have 2 active validators, validator 2 does not have enough power

        saManager.setFederatedPower(validators, publicKeys, powers);

        require(saGetter.isActiveValidator(validators[0]), "not active validator 0");
        require(saGetter.isActiveValidator(validators[1]), "not active validator 1");
        require(!saGetter.isActiveValidator(validators[2]), "2 should not be active validator");

        // change in validator power
        powers[2] = 10001;

        saManager.setFederatedPower(validators, publicKeys, powers);

        confirmChange(validators[0], privKeys[0], validators[1], privKeys[1]);

        require(!saGetter.isActiveValidator(validators[0]), "0 should not be active validator");
        require(saGetter.isActiveValidator(validators[1]), "not active validator 1");
        require(saGetter.isActiveValidator(validators[2]), "not active validator 2");

        /// reduce validator 2 power
        powers[2] = 5000;

        saManager.setFederatedPower(validators, publicKeys, powers);

        confirmChange(validators[2], privKeys[2], validators[1], privKeys[1]);

        require(saGetter.isActiveValidator(validators[0]), "not active validator 0");
        require(saGetter.isActiveValidator(validators[1]), "not active validator 1");
        require(!saGetter.isActiveValidator(validators[2]), "2 should not be active validator");
    }

    function testSubnetActorDiamond_FederatedValidation_worksWithDuplicates() public {
        gatewayAddress = address(gatewayDiamond);

        createSubnetActor(
            gatewayAddress,
            ConsensusType.Fendermint,
            DEFAULT_MIN_VALIDATOR_STAKE,
            DEFAULT_MIN_VALIDATORS,
            DEFAULT_CHECKPOINT_PERIOD,
            DEFAULT_MAJORITY_PERCENTAGE,
            PermissionMode.Federated,
            2
        );

        (address[] memory validators, uint256[] memory privKeys, bytes[] memory publicKeys) = TestUtils.newValidators(
            3
        );
        uint256[] memory powers = new uint256[](3);
        powers[0] = 10000;
        powers[1] = 20000;
        powers[2] = 5000; // we only have 2 active validators, validator 2 does not have enough power

        saManager.setFederatedPower(validators, publicKeys, powers);

        require(saGetter.isActiveValidator(validators[0]), "not active validator 0");
        require(saGetter.isActiveValidator(validators[1]), "not active validator 1");
        require(!saGetter.isActiveValidator(validators[2]), "2 should not be active validator");

        // change in validator power, changing validator 2's power to 10001.

        // store validator 0's data in new variables
        address prevV = validators[0];
        uint256 prevPrivateKey = privKeys[0];

        // creating duplicates of validator 2's data
        validators[0] = validators[2];
        publicKeys[0] = publicKeys[2];

        // the latest validator 2's power, so we have a duplicate of validator 2's power
        powers[0] = 9999; // lower than validator 0's power, will not kick validator 1 off
        powers[2] = 10001; // higher than validator 0's power, will kick validator 1 off

        saManager.setFederatedPower(validators, publicKeys, powers);

        confirmChange(prevV, prevPrivateKey, validators[1], privKeys[1]);

        // we should see validator 0 kicked off
        require(!saGetter.isActiveValidator(prevV), "0 should not be active validator");
        require(saGetter.isActiveValidator(validators[1]), "not active validator 1");
        require(saGetter.isActiveValidator(validators[2]), "not active validator 2");
    }

    function testSubnetActorDiamond_Pausable_SetPaused() public {
        saPauser.pause();
        require(saPauser.paused());

        saPauser.unpause();
        require(!saPauser.paused());
    }

    function testSubnetActorDiamond_Pausable_EnforcedPause() public {
        saPauser.pause();
        uint256 n = 1;
        (address[] memory validators, , bytes[] memory publicKeys) = TestUtils.newValidators(n);

        vm.prank(validators[0]);
        vm.deal(validators[0], 20);
        vm.expectRevert(Pausable.EnforcedPause.selector);
        saManager.join{value: 10}(publicKeys[0]);
    }

    function testSubnetActorDiamond_Pausable_NotOwner() public {
        vm.startPrank(address(1));
        vm.expectRevert(LibDiamond.NotOwner.selector);
        saPauser.pause();
    }

    function testSubnetActorDiamond_Pausable_CannotPauseAgain() public {
        saPauser.pause();

        vm.expectRevert(Pausable.EnforcedPause.selector);
        saPauser.pause();
    }

    function testSubnetActorDiamond_Pausable_CannotUnpauseAgain() public {
        vm.expectRevert(Pausable.ExpectedPause.selector);
        saPauser.unpause();
    }
}
