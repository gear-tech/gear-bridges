// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {IERC1967} from "@openzeppelin/contracts/interfaces/IERC1967.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {Test} from "forge-std/Test.sol";
import {IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {Hasher} from "src/libraries/Hasher.sol";
import {
    GovernancePacker,
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage
} from "src/libraries/packing/GovernancePacker.sol";
import {Base} from "test/Base.sol";

contract WrappedVaraTest is Test, Base {
    using Hasher for VaraMessage;

    using GovernancePacker for PauseProxyMessage;
    using GovernancePacker for UnpauseProxyMessage;
    using GovernancePacker for UpgradeProxyMessage;

    function setUp() public {
        deployBridgeDependsOnEnvironment();
    }

    function test_PauseWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(wrappedVara)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(wrappedVara));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        vm.startPrank(address(erc20Manager));

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        wrappedVara.mint(address(this), 1000);

        vm.stopPrank();
    }

    function test_PauseWithGovernancePauser() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(wrappedVara)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(wrappedVara));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Paused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        vm.startPrank(address(erc20Manager));

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        wrappedVara.mint(address(this), 1000);

        vm.stopPrank();
    }

    function test_PauseUnauthorized() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), wrappedVara.PAUSER_ROLE()
            )
        );
        wrappedVara.pause();
    }

    function test_UnpauseWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(wrappedVara)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(wrappedVara));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.startPrank(address(erc20Manager));

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        // forge-lint: disable-next-item(reentrancy-no-eth)
        wrappedVara.mint(address(this), 1000);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.stopPrank();

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UnpauseProxyMessage({proxy: address(wrappedVara)}).pack()
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        messageHash = message2.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        vm.expectEmit(address(wrappedVara));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Unpaused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), true);

        vm.startPrank(address(erc20Manager));

        wrappedVara.mint(address(this), 1000);

        vm.stopPrank();
    }

    function test_UnpauseWithGovernancePauser() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(wrappedVara)}).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectEmit(address(wrappedVara));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Paused(address(governancePauser));

        // forge-lint: disable-next-item(reentrancy-no-eth)
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.startPrank(address(erc20Manager));

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        // forge-lint: disable-next-item(reentrancy-no-eth)
        wrappedVara.mint(address(this), 1000);

        // forge-lint: disable-next-item(reentrancy-no-eth)
        vm.stopPrank();

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: UnpauseProxyMessage({proxy: address(wrappedVara)}).pack()
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        messageHash = message2.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        vm.expectEmit(address(wrappedVara));
        // forge-lint: disable-next-item(reentrancy-events)
        emit PausableUpgradeable.Unpaused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), true);

        vm.startPrank(address(erc20Manager));

        wrappedVara.mint(address(this), 1000);

        vm.stopPrank();
    }

    function test_UnpauseUnauthorized() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), wrappedVara.PAUSER_ROLE()
            )
        );
        wrappedVara.unpause();
    }

    function test_UpgradeToAndCallWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UpgradeProxyMessage({
                proxy: address(wrappedVara), newImplementation: address(newImplementationMock), data: ""
            }).pack()
        });
        assertEq(messageQueue.isProcessed(message1.nonce), false);

        bytes32 messageHash = message1.hash();

        uint256 blockNumber = currentBlockNumber++;
        bytes32 merkleRoot = messageHash;
        bytes memory proof1 = "";

        vm.expectEmit(address(messageQueue));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(wrappedVara));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IERC1967.Upgraded(address(newImplementationMock));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(
            address(uint160(uint256(vm.load(address(wrappedVara), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );

        // just to get coverage for NewImplementationMock contract

        vm.expectEmit(address(wrappedVara));
        // forge-lint: disable-next-item(reentrancy-events)
        emit IERC1967.Upgraded(address(newImplementationMock));

        wrappedVara.upgradeToAndCall(address(newImplementationMock), "");
        assertEq(
            address(uint160(uint256(vm.load(address(wrappedVara), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );
    }

    function test_UpgradeToAndCallUnauthorized() public {
        vm.startPrank(address(governancePauser));

        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector,
                address(governancePauser),
                wrappedVara.DEFAULT_ADMIN_ROLE()
            )
        );
        wrappedVara.upgradeToAndCall(address(0), "");

        vm.stopPrank();
    }
}
