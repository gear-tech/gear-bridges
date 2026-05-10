// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.35;

import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {IERC1967} from "@openzeppelin/contracts/interfaces/IERC1967.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {Test} from "forge-std/Test.sol";
import {VerifierTestnet} from "src/VerifierTestnet.sol";
import {
    GovernancePacker,
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage
} from "src/interfaces/IGovernance.sol";
import {IMessageHandlerMock} from "src/interfaces/IMessageHandlerMock.sol";
import {Hasher, IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {IVerifierMock} from "src/interfaces/IVerifierMock.sol";
import {Base} from "test/Base.sol";

contract MessageQueueTest is Test, Base {
    using Hasher for VaraMessage;

    using GovernancePacker for PauseProxyMessage;
    using GovernancePacker for UnpauseProxyMessage;
    using GovernancePacker for UpgradeProxyMessage;

    function setUp() public {
        deployBridgeDependsOnEnvironment();
    }

    function testnetVerifierFixture()
        private
        pure
        returns (bytes memory proof, uint256 canonicalBlock, bytes32 merkleRoot)
    {
        proof = bytes(
            hex"19d4cef1d44499f18661c86024492adda2eafa5ce718d0338c6653bf353f803d1712a538d6b2a7f7c91591ebf9e3ee062cbda5fd77b4d0dd21bf8d61c48002883019289c395cafa1f29cb150a42225b66ee6c5ebf8ea84fe9d615abb8128ac291a34cff8f1ee56dbb4ad25284e4e864db85a809278cd84496a07c58378835f920a424f50ce352f41d7d27cf33e0ae31cc7e8cfa559a67e26a08f16a2f1cdf5ea079d8e9d7b9e668b5bde19664e3bed35af688386c6c93498922db88e4b5a02062b240bbfb6e65ce4f4d06415f0047cab6f4d439d218f6f6ab336458bde735f791eb8a39aef494106da910be2a43ccd7fa598817229370d6b05bfcee248053a4d0af4e6deadba0d9f1a92f87e5e65e57c065d9c23c32cc7c8cb08ce49ead2709b2f198063d7bea04c20cf89261ae42035f3b6661c18b8b516c90cb6cf202537560a3e721a70ab8e5feef90ad13b2b043132b5f9ec1ca0ed8b23e73be996b318c4080120074013cd2c301957177257c7125dacacdedbf6aad203b1ee5781450a211073677b3abc886efdc9d43909754686c174940b1c51585a00d5e0bea93e9ef5223c9930bb93b2f324668beb035ea6fcdaed20495fec6649431d6f9c729f91ec1755551a80cfa077db8175371314b9d7de415db8b760f386baa68c52adefafa81dc93e1a0d9528a83feca29f4970d1ec7e81502059df9c671b457a56ed8fa1f410e04d7549207aa42bb5916787b8003f4ddfadaeb089f1800129a98b464a271f12753b7ad7ec1ca60fc5e03a00522906e5e0ce9998eedc26a538dedde8a12c4e04fb72fa98cd636a56212730d27ecf5eb7feb19cf3dcd1e5c9807d0aa7a6848b1d2f943e87d75a44af766187ba0c6d1d89c83491bc2ecedf513757ab4a0cf10824a1b1163f69aaad7eff13ae5deb9dc528cd1a17c5609ee5c5514158a568bb8503fab5ccac261c52d318dc477142f1e8b3d5cee780a4c092e8832d16e0652985287c554b7689b32baed7cb4532437957e08d7db3d9a33c025f38b5fd3a931dd72e1c83c29385d8528c3374f9634d465005bc4603417620c8cc9849da7f83c2852713aa149e09a3acfa1d6b2d2c40f35c13f71ac3169b5dfe5d25125386f9bf0505bdff26ea8790cec9ab5255b3468e62f13d4378fdfb819495e881b58723d85c0eb889d41f1c98f51551d7c31b0e6f190caecc29ad0f4338763c2b8367ce496c2b04806f7b09eb0f145cc7b23e98c1eb6b7e5da0ee3b2470b10bc4e57a5e43de2ed6794c3eb9effaa1d5482c74ff0b59f95e791dfe5bac8a299607287216577c"
        );
        canonicalBlock = 24383731;
        merkleRoot = 0x869ef62b91c490f37173a7dfbaacb3fcee64b4225d7e0977435b054efdcb54b2;
    }

    function test_SubmitMerkleRootWithCanonicalProofFixture() public {
        if (!isFork()) {
            vm.etch(address(verifier), type(VerifierTestnet).runtimeCode);

            (bytes memory proof, uint256 canonicalBlock, bytes32 merkleRoot) = testnetVerifierFixture();

            messageQueue.submitMerkleRoot(canonicalBlock, merkleRoot, proof);

            assertEq(messageQueue.getMerkleRoot(canonicalBlock), merkleRoot);
            assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());
            assertEq(messageQueue.genesisBlock(), canonicalBlock);
            assertEq(messageQueue.maxBlockNumber(), canonicalBlock);
        }
    }

    function test_SubmitMerkleRootRejectsAliasedBlockNumber() public {
        if (!isFork()) {
            vm.etch(address(verifier), type(VerifierTestnet).runtimeCode);

            (bytes memory proof, uint256 canonicalBlock, bytes32 merkleRoot) = testnetVerifierFixture();
            uint256 aliasBlock = canonicalBlock | (1 << 32);

            vm.expectRevert(abi.encodeWithSelector(IMessageQueue.BlockNumberOverflow.selector, aliasBlock));
            messageQueue.submitMerkleRoot(aliasBlock, merkleRoot, proof);

            assertEq(messageQueue.genesisBlock(), 0);
            assertEq(messageQueue.maxBlockNumber(), 0);
            assertEq(messageQueue.getMerkleRoot(aliasBlock), bytes32(0));
            assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), 0);
        }
    }

    function test_PauseWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);
    }

    function test_PauseWithGovernancePauser() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);
    }

    function test_PauseUnauthorized() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), messageQueue.PAUSER_ROLE()
            )
        );
        messageQueue.pause();
    }

    function test_UnpauseWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        VaraMessage memory message3 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UnpauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message3.nonce), false);

        messageHash = message3.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Unpaused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message3, proof2);
        assertEq(messageQueue.isProcessed(message3.nonce), true);
    }

    function test_UnpauseWithGovernancePauser() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        VaraMessage memory message3 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: UnpauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message3.nonce), false);

        messageHash = message3.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Unpaused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message3, proof2);
        assertEq(messageQueue.isProcessed(message3.nonce), true);
    }

    function test_UnpauseUnauthorized() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), messageQueue.PAUSER_ROLE()
            )
        );
        messageQueue.unpause();
    }

    function test_UpgradeToAndCallWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UpgradeProxyMessage({
                proxy: address(messageQueue), newImplementation: address(newImplementationMock), data: ""
            }).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit IERC1967.Upgraded(address(newImplementationMock));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );

        // just to get coverage for NewImplementationMock contract

        vm.expectEmit(address(messageQueue));
        emit IERC1967.Upgraded(address(newImplementationMock));

        messageQueue.upgradeToAndCall(address(newImplementationMock), "");
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );
    }

    function test_UpgradeToAndCallUnauthorized() public {
        vm.startPrank(address(governancePauser));

        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector,
                address(governancePauser),
                messageQueue.DEFAULT_ADMIN_ROLE()
            )
        );
        messageQueue.upgradeToAndCall(address(0), "");

        vm.stopPrank();
    }

    function test_ChallengeRoot() public {
        uint256 blockNumber1 = currentBlockNumber++;
        uint256 blockNumber2 = currentBlockNumber++;
        uint256 blockNumber3 = currentBlockNumber++;
        uint256 blockNumber4 = currentBlockNumber++;

        uint256 blockNumber = blockNumber1;
        bytes32 merkleRoot = bytes32(uint256(0x22)); // valid root, update max block height
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        blockNumber = blockNumber2;
        merkleRoot = bytes32(uint256(0x33)); // invalid root, suspicious address managed to send it

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        blockNumber = blockNumber3;
        merkleRoot = bytes32(uint256(0x44)); // invalid root, suspicious address managed to send it

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();

        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        blockNumber = blockNumber4;
        merkleRoot = messageHash; // valid root, just to check that no one can submit any root now

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        blockNumber = blockNumber2;
        merkleRoot = bytes32(uint256(0xdeadbeef)); // valid root, calculated by emergency stop admin
        bytes32 previousMerkleRoot = bytes32(uint256(0x33));

        // emergency stop admin managed to submit valid root for first challenged block
        // and enabled emergency stop status
        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.EmergencyStopEnabled();

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootDisabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.isEmergencyStopped(), true);

        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestamp(previousMerkleRoot), 0);

        blockNumber = blockNumber3;
        merkleRoot = bytes32(uint256(0xfee1dead)); // valid root, calculated by emergency stop admin
        previousMerkleRoot = bytes32(uint256(0x44));

        // emergency stop admin managed to submit valid root for second challenged block
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestamp(previousMerkleRoot), 0);

        // when all bad roots are removed
        // governance can send message to update MessageQueue and remove emergency stop status
        // emergencyStopAdmin in this case will only accept roots from our RPC node
        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UpgradeProxyMessage({
                proxy: address(messageQueue), newImplementation: address(newImplementationMock), data: ""
            }).pack()
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        messageHash = message2.hash();

        blockNumber = blockNumber3;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        vm.expectEmit(address(messageQueue));
        emit IERC1967.Upgraded(address(newImplementationMock));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(
            address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );

        vm.stopPrank();
    }

    function test_ChallengeRootWithNotEmergencyStopObserver() public {
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopObserver.selector));
        messageQueue.challengeRoot();
    }

    function test_ChallengeRootTwice() public {
        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.warp(vm.getBlockTimestamp() + 1);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();
    }

    function test_ChallengeRootWithPause() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22)); // valid root, update max block height
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(messageQueue)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash; // invalid root, suspicious address managed to send pause

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageQueue));
        emit PausableUpgradeable.Paused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();

        blockNumber = currentBlockNumber++;
        merkleRoot = bytes32(uint256(0x33)); // valid root, just to check that no one can submit any root now

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRoot.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
    }

    function test_DisableChallengeRoot() public {
        vm.startPrank(deploymentArguments.emergencyStopObservers[0]);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootEnabled(vm.getBlockTimestamp() + messageQueue.CHALLENGE_ROOT_DELAY());

        messageQueue.challengeRoot();
        assertEq(messageQueue.isChallengingRoot(), true);

        vm.stopPrank();

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.ChallengeRootDisabled();

        messageQueue.disableChallengeRoot();

        vm.stopPrank();
    }

    function test_DisableChallengeRootWithNotEmergencyStopAdmin() public {
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopAdmin.selector));
        messageQueue.disableChallengeRoot();
    }

    function test_DisableChallengeRootWithChallengeRootNotEnabled() public {
        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.ChallengeRootNotEnabled.selector));
        messageQueue.disableChallengeRoot();

        vm.stopPrank();
    }

    function test_AllowMessageProcessing() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        merkleRoot = bytes32(uint256(0x33));

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.EmergencyStopEnabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MessageProcessingAllowed();

        messageQueue.allowMessageProcessing();

        vm.stopPrank();

        // message is not processed, but anyone can call processMessage now
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootNotFound.selector, blockNumber));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_AllowMessageProcessingWithNotEmergencyStopAdmin() public {
        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.NotEmergencyStopAdmin.selector));
        messageQueue.allowMessageProcessing();
    }

    function test_AllowMessageProcessingWithNotEmergencyStop() public {
        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStopNotEnabled.selector));
        messageQueue.allowMessageProcessing();

        vm.stopPrank();
    }

    function test_SubmitMerkleRoot() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());
    }

    function test_SubmitMerkleRootWithBlockNumberBeforeGenesis() public {
        uint256 blockNumber = messageQueue.genesisBlock();
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        if (blockNumber == 0) {
            blockNumber += 42;

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

            messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

            assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
            assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());
            assertEq(messageQueue.genesisBlock(), blockNumber);
            assertEq(messageQueue.maxBlockNumber(), blockNumber);
        }

        blockNumber -= 1;

        vm.expectRevert(
            abi.encodeWithSelector(
                IMessageQueue.BlockNumberBeforeGenesis.selector, blockNumber, messageQueue.genesisBlock()
            )
        );
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootWithBlockNumberTooFar() public {
        uint256 blockNumber = messageQueue.genesisBlock();
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        if (blockNumber == 0) {
            blockNumber += 42;

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

            messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

            assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
            assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());
            assertEq(messageQueue.genesisBlock(), blockNumber);
            assertEq(messageQueue.maxBlockNumber(), blockNumber);
        }

        blockNumber = messageQueue.maxBlockNumber() + messageQueue.MAX_BLOCK_DISTANCE() + 1;

        vm.expectRevert(
            abi.encodeWithSelector(
                IMessageQueue.BlockNumberTooFar.selector,
                blockNumber,
                messageQueue.maxBlockNumber() + messageQueue.MAX_BLOCK_DISTANCE()
            )
        );
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootTwice() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootAlreadySet.selector, blockNumber));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootWithInvalidProof() public {
        IVerifierMock(address(verifier)).setValue(false);

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.InvalidPlonkProof.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);
    }

    function test_SubmitMerkleRootWithEmergencyStop() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        bytes32 previousMerkleRoot = merkleRoot;
        merkleRoot = bytes32(uint256(0x33));

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.EmergencyStopEnabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        assertEq(messageQueue.isEmergencyStopped(), true);

        assertEq(messageQueue.getMerkleRoot(blockNumber), bytes32(0));
        assertEq(messageQueue.getMerkleRootTimestamp(previousMerkleRoot), 0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.startPrank(deploymentArguments.emergencyStopAdmin);

        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageHandlerMock));
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);

        vm.stopPrank();
    }

    function test_ProcessMessage() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageHandlerMock));
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
    }

    function test_ProcessMessageWithEmergencyStop() public {
        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = bytes32(uint256(0x22));
        bytes memory proof = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        assertEq(messageQueue.getMerkleRoot(blockNumber), merkleRoot);
        assertEq(messageQueue.getMerkleRootTimestamp(merkleRoot), vm.getBlockTimestamp());

        merkleRoot = bytes32(uint256(0x33));

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.EmergencyStopEnabled();

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.EmergencyStop.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_ProcessMessageWithMessageAlreadyProcessed() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(messageHandlerMock));
        emit IMessageHandlerMock.MessageHandled(message.source, message.payload);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, message.nonce));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
    }

    function test_ProcessMessageWithMerkleRootNotSet() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        uint256 blockNumber = currentBlockNumber++;
        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootNotFound.selector, blockNumber));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_ProcessMessageWithMerkleRootDelayNotPassed() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof = new bytes32[](0);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MerkleRootDelayNotPassed.selector, blockNumber));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_ProcessMessageWithInvalidMerkleProof() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(messageHandlerMock),
            payload: hex"33"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](1);
        proof2[0] = bytes32(uint256(0xbad));

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.InvalidMerkleProof.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }
}
