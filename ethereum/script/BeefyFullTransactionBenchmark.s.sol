// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VaraQueueRootVerifier} from "src/VaraQueueRootVerifier.sol";
import {BeefyClient} from "src/beefy/BeefyClient.sol";
import {VaraBridgeMetadata} from "src/beefy/VaraBridgeMetadata.sol";
import {CircleToken} from "src/erc20/CircleToken.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {BeefyFixtureTest} from "test/BeefyInterop.t.sol";

/// @dev Local-only fixture broadcaster. The Python entrypoint owns the Anvil and
/// measures mined receipts, not the script's shared/warm execution context.
contract BeefyFullTransactionBenchmark is BeefyFixtureTest {
    BeefyClient internal client;
    VaraQueueRootVerifier internal verifier;
    MessageQueue internal queue;
    address internal constant SENDER = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
    bytes32 internal constant VFT_MANAGER = bytes32(uint256(0x909));

    function runCase(uint256 index) public {
        require(block.chainid == 31337, "local benchmark chain only");
        require(index >= 10 && index <= 12, "expected 59/150/256 fixture");
        fixtures = vm.readFile("test/fixtures/beefy-interop.json");
        _benchmarkAuthoritySet(index);
    }

    function run() public {
        require(block.chainid == 31337, "local benchmark chain only");
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
        (uint128 current,,,) = target.currentValidatorSet();
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

    function _prepareQueue() internal {
        address predictedQueue = vm.computeCreateAddress(SENDER, vm.getNonce(SENDER) + 5);
        vm.startBroadcast(SENDER);
        client = newClient(0, block.chainid, predictedQueue);
        MessageQueue implementation = new MessageQueue();
        GovernanceAdmin admin = new GovernanceAdmin(
            bytes32(uint256(33)), WrappedVara(address(0)), MessageQueue(address(0)), ERC20Manager(address(0))
        );
        GovernancePauser pauser = new GovernancePauser(
            bytes32(uint256(44)), WrappedVara(address(0)), MessageQueue(address(0)), ERC20Manager(address(0))
        );
        verifier = new VaraQueueRootVerifier(client, predictedQueue, block.chainid);
        queue = MessageQueue(
            address(
                new ERC1967Proxy(
                    address(implementation),
                    abi.encodeCall(
                        MessageQueue.initializeBeefy,
                        (IGovernance(address(admin)), IGovernance(address(pauser)), SENDER, new address[](0), verifier)
                    )
                )
            )
        );
        require(address(queue) == predictedQueue, "queue prediction mismatch");
        CircleToken token = new CircleToken(SENDER);
        IERC20Manager.TokenInfo[] memory tokens = new IERC20Manager.TokenInfo[](1);
        tokens[0] = IERC20Manager.TokenInfo(address(token), IERC20Manager.TokenType.Ethereum);
        ERC20Manager manager = ERC20Manager(
            address(
                new ERC1967Proxy(
                    address(new ERC20Manager()),
                    abi.encodeCall(
                        ERC20Manager.initialize,
                        (IGovernance(address(admin)), IGovernance(address(pauser)), queue, VFT_MANAGER, tokens)
                    )
                )
            )
        );
        token.mint(SENDER, 30);
        token.approve(address(manager), 30);
        manager.requestBridging(address(token), 30, VFT_MANAGER);
        VaraMessage memory message = _message(address(manager), address(token));
        bytes32 root = keccak256(abi.encodePacked(message.nonce, message.source, message.destination, message.payload));
        QueueProof memory p = customQueueProofFor(client, 100, 103, root, 1_800_000_000_000, 1_800_000_003_000);
        queue.submitMerkleRoot(100, root, encodeProof(p));
        vm.stopBroadcast();
        assertEq(queue.getMerkleRoot(100), root);
        assertEq(token.balanceOf(address(manager)), 30);
    }

    function executeMessage(address queueAddress, address managerAddress, address tokenAddress) public {
        require(block.chainid == 31337, "local benchmark chain only");
        MessageQueue target = MessageQueue(queueAddress);
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
