// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {DeploymentBase} from "script/DeploymentBase.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {CircleToken} from "src/erc20/CircleToken.sol";
import {VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {BeefyFixtureTest} from "test/BeefyInterop.t.sol";

/// @dev Local-only fixture broadcaster. The Python entrypoint owns the Anvil and
/// measures mined receipts, not the script's shared/warm execution context.
contract BeefyFullTransactionBenchmark is BeefyFixtureTest, DeploymentBase {
    address internal constant SENDER = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
    bytes32 internal constant VFT_MANAGER = bytes32(uint256(0x909));

    // Synthetic authorities measure transactions; they never approve a native source.
    // The exemption exists only in this test-only 31337 script, not relay writers.
    function _localDeploymentFixture() internal view override returns (bool) {
        return block.chainid == 31337 && vm.envOr("BEEFY_BENCHMARK_OWNED_RUNNER", false);
    }

    function runCase(uint256 index) public {
        require(block.chainid == 31337, "local benchmark chain only");
        require(index >= 10 && index <= 12, "expected 59/150/256 fixture");
        _requireOwnedRunner();
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
        _benchmarkAuthoritySet(index);
    }

    function run() public {
        require(block.chainid == 31337, "local benchmark chain only");
        _requireOwnedRunner();
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
        _prepareQueue();
    }

    function _benchmarkAuthoritySet(uint256 index) internal {
        vm.startBroadcast(SENDER);
        BeefyClient target = newClient(index);
        // This first update uses the Rust-generated, already-signed fixture.
        acceptFixture(target, index);
        vm.stopBroadcast();
        assertEq(target.latestMMRRoot(), fixtureHash(index, "mmrRoot"));

        // Subsequent snapshots use the existing fixture authorities and genuine
        // ECDSA signatures, as in testInteractiveAuthenticatedHandover. These
        // are explicit synthetic local source updates, not production proofs.
        uint256 rotatedIndex = index == 12 ? 10 : index + 1;
        _nextUpdate(target, index, 103, 0, 1, index);
        _nextUpdate(target, index, 104, 1, 2, rotatedIndex);
        _nextUpdate(target, rotatedIndex, 105, 2, 3, index);
        (uint128 current,,) = target.currentValidatorSet();
        assertEq(current, 2);
    }

    function _nextUpdate(
        BeefyClient target,
        uint256 index,
        uint32 height,
        uint64 setId,
        uint64 nextSetId,
        uint256 nextIndex
    ) internal {
        VaraBridgeMetadata.Snapshot memory s = snapshot(index, true);
        s.bridgeDomain = target.bridgeDomain();
        BeefyClient.MMRLeaf memory leaf = leafForSnapshot(
            s,
            height - 1,
            nextSetId,
            uint32(fixtureUint(nextIndex, "validatorCount")),
            fixtureHash(nextIndex, "authorityRoot")
        );
        bytes32 root = keccak256(leafBytes(leaf));
        BeefyClient.Commitment memory c = commitment(index, height, setId, root);
        BeefyClient.ValidatorProof[] memory proofs = signedProofs(target, c, index, "");
        vm.startBroadcast(SENDER);
        target.submitFiatShamir(c, allSigners(index), proofs, leaf, s, new bytes32[](0), 0);
        vm.stopBroadcast();
        assertEq(target.latestMMRRoot(), root);
        assertEq(target.latestBeefyBlock(), height);
    }

    function _requireOwnedRunner() private view {
        require(vm.envOr("BEEFY_BENCHMARK_OWNED_RUNNER", false), "owned benchmark runner required");
        require(vm.addr(vm.envUint("PRIVATE_KEY")) == SENDER, "disposable Anvil key required");
    }

    function _prepareQueue() internal {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address[] memory proposers = new address[](1);
        proposers[0] = SENDER;
        vm.startBroadcast(privateKey);
        TimelockController recovery = new TimelockController(1 hours, proposers, proposers, address(0));
        vm.stopBroadcast();

        // Reuse the full environment and its guarded, client-first CREATE prediction.
        vm.setEnv("DEPLOYMENT_MODE", "fresh-beefy");
        vm.setEnv("DEPLOY_TEST_TOKENS", "true");
        vm.setEnv("CIRCLE_TOKEN", vm.toString(address(0)));
        vm.setEnv("TETHER_TOKEN", vm.toString(address(0)));
        vm.setEnv("WRAPPED_ETHER", vm.toString(address(0)));
        vm.setEnv("WRAPPED_BITCOIN", vm.toString(address(0)));
        vm.setEnv("VFT_MANAGER", vm.toString(VFT_MANAGER));
        vm.setEnv("GOVERNANCE_ADMIN", vm.toString(bytes32(uint256(33))));
        vm.setEnv("GOVERNANCE_PAUSER", vm.toString(bytes32(uint256(44))));
        vm.setEnv("EMERGENCY_STOP_ADMIN", vm.toString(SENDER));
        vm.setEnv("EMERGENCY_STOP_OBSERVERS", vm.toString(SENDER));
        vm.setEnv("BRIDGING_PAYMENT_FEE", "1");
        vm.setEnv("BEEFY_RECOVERY_TIMELOCK", vm.toString(address(recovery)));
        address predictedQueue = _predictBeefyMessageQueue(SENDER, _tokenOverridesFromEnvironment());
        bytes32 sourceDomain = fixtureHash(0, "sourceDomain");
        vm.setEnv("BEEFY_SOURCE_DOMAIN", vm.toString(sourceDomain));
        vm.setEnv(
            "BEEFY_BRIDGE_DOMAIN",
            vm.toString(
                keccak256(
                    abi.encodePacked(
                        "vara/gear-eth-bridge-domain/v2", sourceDomain, bytes32(block.chainid), predictedQueue
                    )
                )
            )
        );
        vm.setEnv("BEEFY_MMR_START_BLOCK", vm.toString(fixtureUint(0, "mmrStartBlock")));
        vm.setEnv("BEEFY_INITIAL_BLOCK", vm.toString(fixtureUint(0, "mmrStartBlock") + 1));
        vm.setEnv("BEEFY_INITIAL_SOURCE_TIMESTAMP_MS", vm.toString(bootstrapTimestamp(0)));
        vm.setEnv("BEEFY_CURRENT_ID", "0");
        vm.setEnv("BEEFY_CURRENT_LENGTH", vm.toString(fixtureUint(0, "validatorCount")));
        vm.setEnv("BEEFY_CURRENT_ROOT", vm.toString(fixtureHash(0, "authorityRoot")));
        vm.setEnv("BEEFY_NEXT_ID", "1");
        vm.setEnv("BEEFY_NEXT_LENGTH", vm.toString(fixtureUint(0, "validatorCount")));
        vm.setEnv("BEEFY_NEXT_ROOT", vm.toString(fixtureHash(0, "authorityRoot")));
        deployBridgeFromEnvironment();

        CircleToken token = CircleToken(address(circleToken));
        vm.startBroadcast(privateKey);
        token.mint(SENDER, 30);
        token.approve(address(erc20Manager), 30);
        erc20Manager.requestBridging(address(token), 30, VFT_MANAGER);
        VaraMessage memory message = _message(address(erc20Manager), address(token));
        bytes32 root = keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
        QueueProof memory p = customQueueProofFor(beefyClient, 100, 103, root, 1_800_000_000_000, 1_800_000_003_000);
        messageQueue.submitMerkleRoot(100, root, encodeProof(p));
        vm.stopBroadcast();
        assertEq(messageQueue.getMerkleRoot(100), root);
        assertEq(token.balanceOf(address(erc20Manager)), 30);
    }

    function executeMessage(address queueAddress, address managerAddress, address tokenAddress) public {
        require(block.chainid == 31337, "local benchmark chain only");
        MessageQueue target = MessageQueue(queueAddress);
        _requireOwnedRunner();
        require(!target.isProcessed(702), "benchmark message already processed");
        vm.startBroadcast(SENDER);
        target.processMessage(100, 1, 0, _message(managerAddress, tokenAddress), new bytes32[](0));
        vm.stopBroadcast();
        assertTrue(target.isProcessed(702));
        assertEq(CircleToken(tokenAddress).balanceOf(managerAddress), 20);
        assertEq(CircleToken(tokenAddress).balanceOf(SENDER), 10);
    }

    function _message(address manager, address token) internal pure returns (VaraMessage memory) {
        return VaraMessage(702, VFT_MANAGER, manager, abi.encodePacked(bytes32(uint256(1)), SENDER, token, uint256(10)));
    }
}
