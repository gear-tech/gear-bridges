// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {Base} from "./Base.sol";
import {IAccessControl} from "@openzeppelin/contracts/access/IAccessControl.sol";
import {IERC1967} from "@openzeppelin/contracts/interfaces/IERC1967.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {IERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {UnsafeUpgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {TetherToken} from "src/erc20/TetherToken.sol";
import {IBridgingPayment} from "src/interfaces/IBridgingPayment.sol";
import {
    IERC20Manager,
    TransferMessage,
    AddVftManagerMessage,
    RegisterEthereumTokenMessage,
    RegisterGearTokenMessage,
    ERC20ManagerPacker
} from "src/interfaces/IERC20Manager.sol";
import {IERC20Mintable} from "src/interfaces/IERC20Mintable.sol";
import {
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage,
    GovernancePacker
} from "src/interfaces/IGovernance.sol";
import {VaraMessage, IMessageQueue, Hasher} from "src/interfaces/IMessageQueue.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";

contract ERC20ManagerTest is Test, Base {
    using Hasher for VaraMessage;

    using GovernancePacker for PauseProxyMessage;
    using GovernancePacker for UnpauseProxyMessage;
    using GovernancePacker for UpgradeProxyMessage;

    using ERC20ManagerPacker for TransferMessage;
    using ERC20ManagerPacker for AddVftManagerMessage;
    using ERC20ManagerPacker for RegisterEthereumTokenMessage;
    using ERC20ManagerPacker for RegisterGearTokenMessage;

    function setUp() public {
        deployBridgeFromConstants();
    }

    function test_InitializeWithInvalidTokenType() public {
        IERC20Manager.TokenInfo[] memory tokens = new IERC20Manager.TokenInfo[](1);
        tokens[0] = IERC20Manager.TokenInfo(address(0x11), IERC20Manager.TokenType.Unknown);

        address implementation = address(new ERC20Manager());
        vm.expectRevert(IERC20Manager.InvalidTokenType.selector);
        UnsafeUpgrades.deployUUPSProxy(
            implementation,
            abi.encodeCall(
                ERC20Manager.initialize,
                (governanceAdmin, governancePauser, address(messageQueue), deploymentArguments.vftManager, tokens)
            )
        );
    }

    function test_PauseWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(erc20Manager)}).pack()
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

        vm.expectEmit(address(erc20Manager));
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        address token = address(circleToken);
        uint256 amount = 100 * (10 ** circleToken.decimals());
        bytes32 to = bytes32(uint256(0x11));
        address bridgingPayment_ = address(bridgingPayment);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridging(token, amount, to);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridgingPayingFee(token, amount, to, bridgingPayment_);

        (uint256 deadline, uint8 v, bytes32 r, bytes32 s) = (0, 0, 0, 0);
        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridgingWithPermit(token, amount, to, deadline, v, r, s);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridgingPayingFeeWithPermit(token, amount, to, deadline, v, r, s, bridgingPayment_);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.createBridgingPayment(deploymentArguments.bridgingPaymentFee);
    }

    function test_PauseWithGovernancePauser() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(erc20Manager)}).pack()
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

        vm.expectEmit(address(erc20Manager));
        emit PausableUpgradeable.Paused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        address token = address(circleToken);
        uint256 amount = 100 * (10 ** circleToken.decimals());
        bytes32 to = bytes32(uint256(0x11));
        address bridgingPayment_ = address(bridgingPayment);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridging(token, amount, to);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridgingPayingFee(token, amount, to, bridgingPayment_);

        (uint256 deadline, uint8 v, bytes32 r, bytes32 s) = (0, 0, 0, 0);
        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridgingWithPermit(token, amount, to, deadline, v, r, s);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridgingPayingFeeWithPermit(token, amount, to, deadline, v, r, s, bridgingPayment_);

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.createBridgingPayment(deploymentArguments.bridgingPaymentFee);
    }

    function test_PauseUnauthorized() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), erc20Manager.PAUSER_ROLE()
            )
        );
        erc20Manager.pause();
    }

    function test_UnpauseWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: PauseProxyMessage({proxy: address(erc20Manager)}).pack()
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

        vm.expectEmit(address(erc20Manager));
        emit PausableUpgradeable.Paused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        address token = address(circleToken);
        uint256 amount = 100 * (10 ** circleToken.decimals());
        bytes32 to = bytes32(uint256(0x11));

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridging(token, amount, to);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UnpauseProxyMessage({proxy: address(erc20Manager)}).pack()
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        messageHash = message2.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

        vm.expectEmit(address(erc20Manager));
        emit PausableUpgradeable.Unpaused(address(governanceAdmin));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), true);
    }

    function test_UnpauseWithGovernancePauser() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: PauseProxyMessage({proxy: address(erc20Manager)}).pack()
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

        vm.expectEmit(address(erc20Manager));
        emit PausableUpgradeable.Paused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        address token = address(circleToken);
        uint256 amount = 100 * (10 ** circleToken.decimals());
        bytes32 to = bytes32(uint256(0x11));

        vm.expectRevert(abi.encodeWithSelector(PausableUpgradeable.EnforcedPause.selector));
        erc20Manager.requestBridging(token, amount, to);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: governancePauser.governance(),
            destination: address(governancePauser),
            payload: UnpauseProxyMessage({proxy: address(erc20Manager)}).pack()
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        messageHash = message2.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_PAUSER_MESSAGE_DELAY());

        vm.expectEmit(address(erc20Manager));
        emit PausableUpgradeable.Unpaused(address(governancePauser));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), true);
    }

    function test_UnpauseUnauthorized() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector, address(this), erc20Manager.PAUSER_ROLE()
            )
        );
        erc20Manager.unpause();
    }

    function test_UpgradeToAndCallWithGovernanceAdmin() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(governanceAdmin),
            payload: UpgradeProxyMessage({
                proxy: address(erc20Manager),
                newImplementation: address(newImplementationMock),
                data: ""
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

        vm.expectEmit(address(erc20Manager));
        emit IERC1967.Upgraded(address(newImplementationMock));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(
            address(uint160(uint256(vm.load(address(erc20Manager), ERC1967Utils.IMPLEMENTATION_SLOT)))),
            address(newImplementationMock)
        );
    }

    function test_UpgradeToAndCallUnauthorized() public {
        vm.startPrank(address(governancePauser));

        vm.expectRevert(
            abi.encodeWithSelector(
                IAccessControl.AccessControlUnauthorizedAccount.selector,
                address(governancePauser),
                erc20Manager.DEFAULT_ADMIN_ROLE()
            )
        );
        erc20Manager.upgradeToAndCall(address(0), "");

        vm.stopPrank();
    }

    function test_RequestBridgingWithEthereumToken() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        address token = address(circleToken);
        uint256 amount = 100 * (10 ** circleToken.decimals());
        bytes32 to = 0;

        IERC20Mintable(address(circleToken)).mint(deploymentArguments.deployerAddress, amount);
        circleToken.approve(address(erc20Manager), amount);

        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.BridgingRequested(deploymentArguments.deployerAddress, to, token, amount);

        erc20Manager.requestBridging(token, amount, to);

        assertEq(circleToken.balanceOf(deploymentArguments.deployerAddress), 0);
        assertEq(circleToken.balanceOf(address(erc20Manager)), amount);

        vm.stopPrank();
    }

    function test_RequestBridgingWithEthereumTokenWithZeroAmount() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        address token = address(circleToken);
        uint256 amount = 0;
        bytes32 to = 0;

        IERC20Mintable(address(circleToken)).mint(deploymentArguments.deployerAddress, amount);
        circleToken.approve(address(erc20Manager), amount);

        vm.expectRevert(IERC20Manager.InvalidAmount.selector);
        erc20Manager.requestBridging(token, amount, to);

        vm.stopPrank();
    }

    function test_RequestBridgingWithGearToken() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        address token = address(wrappedVara);
        uint256 amount = 100 * (10 ** wrappedVara.decimals());
        bytes32 to = 0;

        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: deploymentArguments.vftManager,
            destination: address(erc20Manager),
            payload: TransferMessage({
                sender: to,
                receiver: deploymentArguments.deployerAddress,
                token: token,
                amount: amount
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

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        uint256 totalLeaves = 1;
        uint256 leafIndex = 0;
        bytes32[] memory proof2 = new bytes32[](0);

        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.Bridged(to, deploymentArguments.deployerAddress, token, amount);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);
        assertEq(wrappedVara.balanceOf(deploymentArguments.deployerAddress), amount);

        wrappedVara.approve(address(erc20Manager), amount);

        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.BridgingRequested(deploymentArguments.deployerAddress, to, token, amount);

        erc20Manager.requestBridging(token, amount, to);
        assertEq(wrappedVara.balanceOf(deploymentArguments.deployerAddress), 0);

        vm.stopPrank();
    }

    function test_RequestBridgingWithInvalidTokenType() public {
        address token = address(0);
        uint256 amount = 1;
        bytes32 to = 0;

        vm.expectRevert(IERC20Manager.InvalidTokenType.selector);
        erc20Manager.requestBridging(token, amount, to);
    }

    function test_RequestBridgingPayingFee() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        address token = address(tetherToken);
        uint256 amount = 100 * (10 ** tetherToken.decimals());
        bytes32 to = 0;
        address bridgingPayment_ = address(bridgingPayment);

        IERC20Mintable(address(tetherToken)).mint(deploymentArguments.deployerAddress, amount);
        tetherToken.approve(address(erc20Manager), amount);

        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.BridgingRequested(deploymentArguments.deployerAddress, to, token, amount);

        erc20Manager.requestBridgingPayingFee{value: deploymentArguments.bridgingPaymentFee}(
            token, amount, to, bridgingPayment_
        );

        assertEq(tetherToken.balanceOf(deploymentArguments.deployerAddress), 0);
        assertEq(tetherToken.balanceOf(address(erc20Manager)), amount);

        vm.stopPrank();
    }

    function test_RequestBridgingPayingFeeWithInvalidBridgingPayment() public {
        address token = address(circleToken);
        uint256 amount = 100 * (10 ** circleToken.decimals());
        bytes32 to = 0;
        address bridgingPayment_ = address(0);

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidBridgingPayment.selector, bridgingPayment_));
        erc20Manager.requestBridgingPayingFee(token, amount, to, bridgingPayment_);
    }

    function test_RequestBridgingPayingFeeWithIncorrectFeeAmount() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        address token = address(tetherToken);
        uint256 amount = 100 * (10 ** tetherToken.decimals());
        bytes32 to = 0;
        address bridgingPayment_ = address(bridgingPayment);

        IERC20Mintable(address(tetherToken)).mint(deploymentArguments.deployerAddress, amount);
        tetherToken.approve(address(erc20Manager), amount);

        vm.expectRevert(IBridgingPayment.IncorrectFeeAmount.selector);
        erc20Manager.requestBridgingPayingFee(token, amount, to, bridgingPayment_);

        vm.stopPrank();
    }

    function test_RequestBridgingPayingFeeWithPermit() public {
        (address owner, uint256 ownerPrivateKey) = makeAddrAndKey("owner");
        uint256 value = 100 * (10 ** circleToken.decimals());

        vm.startPrank(deploymentArguments.deployerAddress);

        vm.deal(owner, deploymentArguments.bridgingPaymentFee);
        IERC20Mintable(address(circleToken)).mint(owner, value);

        vm.stopPrank();

        vm.startPrank(owner);

        address spender = address(erc20Manager);
        uint256 nonce = IERC20Permit(address(circleToken)).nonces(owner);
        uint256 deadline = vm.getBlockTimestamp() + 1;

        bytes32 structHash = keccak256(
            abi.encode(
                keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"),
                owner,
                spender,
                value,
                nonce,
                deadline
            )
        );
        bytes32 hash =
            MessageHashUtils.toTypedDataHash(IERC20Permit(address(circleToken)).DOMAIN_SEPARATOR(), structHash);

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ownerPrivateKey, hash);

        address token = address(circleToken);
        bytes32 to = 0;
        address bridgingPayment_ = address(bridgingPayment);

        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.BridgingRequested(owner, to, token, value);

        erc20Manager.requestBridgingPayingFeeWithPermit{value: deploymentArguments.bridgingPaymentFee}(
            token, value, to, deadline, v, r, s, bridgingPayment_
        );

        vm.stopPrank();
    }

    function test_RequestBridgingPayingFeeWithPermitWithInvalidBridgingPayment() public {
        address token = address(circleToken);
        uint256 amount = 100 * (10 ** circleToken.decimals());
        bytes32 to = 0;
        address bridgingPayment_ = address(0);
        (uint256 deadline, uint8 v, bytes32 r, bytes32 s) = (0, 0, 0, 0);

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidBridgingPayment.selector, bridgingPayment_));
        erc20Manager.requestBridgingPayingFeeWithPermit(token, amount, to, deadline, v, r, s, bridgingPayment_);
    }

    function test_HandleMessageWithInvalidSender() public {
        bytes32 source = bytes32(uint256(0x11));
        bytes memory payload = "";

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidSender.selector));
        erc20Manager.handleMessage(source, payload);
    }

    function test_HandleMessageWithTransferMessageAndNotEnoughPayload() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: deploymentArguments.vftManager,
            destination: address(erc20Manager),
            payload: ""
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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithTransferMessageAndInvalidTokenType() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: deploymentArguments.vftManager,
            destination: address(erc20Manager),
            payload: TransferMessage({sender: 0, receiver: address(0), token: address(0), amount: 0}).pack()
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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidTokenType.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithTransferMessageAndTokenTypeEthereum() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        address token = address(circleToken);
        uint256 amount = 100 * (10 ** circleToken.decimals());
        bytes32 to = 0;
        IERC20Mintable(address(circleToken)).mint(address(erc20Manager), amount);
        assertEq(circleToken.balanceOf(address(erc20Manager)), amount);

        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: deploymentArguments.vftManager,
            destination: address(erc20Manager),
            payload: TransferMessage({
                sender: to,
                receiver: deploymentArguments.deployerAddress,
                token: token,
                amount: amount
            }).pack()
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

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
        assertEq(circleToken.balanceOf(deploymentArguments.deployerAddress), amount);

        vm.stopPrank();
    }

    function test_HandleMessageWithGovernanceMessageAndNotEnoughPayload() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: ""
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithInvalidSource() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: bytes32(uint256(0x22)),
            destination: address(erc20Manager),
            payload: ""
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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidSource.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithGovernanceMessageAndInvalidDiscriminant() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: hex"ff"
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithGovernanceMessageAddVftManager() public {
        bytes32 newVftManager = bytes32(uint256(0x22));
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: AddVftManagerMessage({vftManager: newVftManager}).pack()
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

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

        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.VftManagerAdded(newVftManager);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), true);
    }

    function test_HandleMessageWithGovernanceMessageAddVftManagerWithInvalidSize() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: abi.encodePacked(uint8(0x00)) // ERC20Manager.ADD_VFT_MANAGER = 0x00
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithGovernanceMessageRegisterEthereumToken() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        TetherToken newTetherToken = new TetherToken(deploymentArguments.deployerAddress);

        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: RegisterEthereumTokenMessage({token: address(newTetherToken)}).pack()
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

        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.EthereumTokenRegistered(address(newTetherToken));

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        address token = address(newTetherToken);
        uint256 amount = 100 * (10 ** newTetherToken.decimals());
        bytes32 to = 0;
        address bridgingPayment_ = address(bridgingPayment);

        newTetherToken.mint(deploymentArguments.deployerAddress, amount);
        newTetherToken.approve(address(erc20Manager), amount);

        assertEq(newTetherToken.balanceOf(deploymentArguments.deployerAddress), amount);

        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.BridgingRequested(deploymentArguments.deployerAddress, to, token, amount);

        erc20Manager.requestBridgingPayingFee{value: deploymentArguments.bridgingPaymentFee}(
            token, amount, to, bridgingPayment_
        );

        assertEq(newTetherToken.balanceOf(deploymentArguments.deployerAddress), 0);
        assertEq(newTetherToken.balanceOf(address(erc20Manager)), amount);

        vm.stopPrank();
    }

    function test_HandleMessageWithGovernanceMessageRegisterEthereumTokenWithInvalidSize() public {
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: abi.encodePacked(uint8(0x01)) // TokenType.Ethereum = 0x01
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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), false);
    }

    function test_HandleMessageWithGovernanceMessageRegisterGearToken() public {
        vm.startPrank(deploymentArguments.deployerAddress);

        // max length is 31 bytes
        string memory tokenName = "MyToken________________________";
        string memory tokenSymbol = "MTK____________________________";
        uint8 tokenDecimals = 18;
        VaraMessage memory message1 = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: RegisterGearTokenMessage({tokenName: tokenName, tokenSymbol: tokenSymbol, tokenDecimals: tokenDecimals})
                .pack()
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

        address token = vm.computeCreateAddress(address(erc20Manager), vm.getNonce(address(erc20Manager)));
        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.GearTokenRegistered(token, tokenName, tokenSymbol, tokenDecimals);

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
        assertEq(messageQueue.isProcessed(message1.nonce), true);

        uint256 amount = 100 * (10 ** IERC20Metadata(token).decimals());
        bytes32 to = 0;
        address bridgingPayment_ = address(bridgingPayment);

        VaraMessage memory message2 = VaraMessage({
            nonce: messageNonce++,
            source: deploymentArguments.vftManager,
            destination: address(erc20Manager),
            payload: TransferMessage({
                sender: to,
                receiver: deploymentArguments.deployerAddress,
                token: token,
                amount: amount
            }).pack()
        });
        assertEq(messageQueue.isProcessed(message2.nonce), false);

        messageHash = message2.hash();

        blockNumber = currentBlockNumber++;
        merkleRoot = messageHash;

        vm.expectEmit(address(messageQueue));
        emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

        messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

        vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);
        assertEq(messageQueue.isProcessed(message2.nonce), true);

        assertEq(IERC20Metadata(token).balanceOf(deploymentArguments.deployerAddress), amount);
        IERC20Metadata(token).approve(address(erc20Manager), amount);

        vm.expectEmit(address(erc20Manager));
        emit IERC20Manager.BridgingRequested(deploymentArguments.deployerAddress, to, token, amount);

        erc20Manager.requestBridgingPayingFee{value: deploymentArguments.bridgingPaymentFee}(
            token, amount, to, bridgingPayment_
        );

        assertEq(IERC20Metadata(token).balanceOf(deploymentArguments.deployerAddress), 0);

        vm.stopPrank();
    }

    function test_HandleMessageWithGovernanceMessageRegisterGearTokenWithInvalidSize() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: abi.encodePacked(uint8(0x02)) // TokenType.Gear = 0x02
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithGovernanceMessageRegisterGearTokenWithInvalidTokenNameLength() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: abi.encodePacked(uint8(0x02), bytes32(0), bytes32(0), uint8(0)) // TokenType.Gear = 0x02
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }

    function test_HandleMessageWithGovernanceMessageRegisterGearTokenWithInvalidTokenSymbolLength() public {
        VaraMessage memory message = VaraMessage({
            nonce: messageNonce++,
            source: governanceAdmin.governance(),
            destination: address(erc20Manager),
            payload: abi.encodePacked(uint8(0x02), bytes32(uint256(1 << 248)), bytes32(0), uint8(0)) // TokenType.Gear = 0x02
        });
        assertEq(messageQueue.isProcessed(message.nonce), false);

        bytes32 messageHash = message.hash();

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

        vm.expectRevert(abi.encodeWithSelector(IERC20Manager.InvalidPayload.selector));
        messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message, proof2);
        assertEq(messageQueue.isProcessed(message.nonce), false);
    }
}
