// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Test} from "forge-std/Test.sol";
import {MessageHandler} from "src/MessageHandler.sol";

contract MessageHandlerTest is Test {
    address private constant QUEUE = address(0x1234);
    address private constant SENDER = address(0x5678);
    bytes32 private constant SOURCE = bytes32(uint256(0xabcdef));
    bytes32 private constant ID = bytes32(uint256(1));
    MessageHandler private handler;

    event MessageRequested(
        bytes32 indexed applicationId, address indexed sender, bytes32 indexed destination, bytes payload
    );
    event MessageHandled(bytes32 indexed source, bytes32 indexed applicationId, bytes payload);

    function setUp() public {
        handler = new MessageHandler(QUEUE, SOURCE, SENDER);
    }

    function testConstructorRejectsUnboundIdentities() public {
        vm.expectRevert(MessageHandler.InvalidPayload.selector);
        new MessageHandler(address(0), SOURCE, SENDER);
        vm.expectRevert(MessageHandler.InvalidPayload.selector);
        new MessageHandler(QUEUE, bytes32(0), SENDER);
        vm.expectRevert(MessageHandler.InvalidPayload.selector);
        new MessageHandler(QUEUE, SOURCE, address(0));
    }

    function testQueueAndSourceAreAuthenticatedBeforeEffects() public {
        vm.expectRevert(MessageHandler.NotQueue.selector);
        handler.handleMessage(SOURCE, abi.encodePacked(ID, hex"0001ff70696e6700"));
        vm.prank(QUEUE);
        vm.expectRevert(MessageHandler.WrongSource.selector);
        handler.handleMessage(bytes32(uint256(2)), abi.encodePacked(ID, hex"0001ff70696e6700"));
        assertFalse(handler.received(ID));
        assertEq(handler.payloadOf(ID), bytes(""));
    }

    function testSenderAndRequestReplayAreAuthenticated() public {
        vm.expectRevert(MessageHandler.NotSender.selector);
        handler.sendMessage(ID, hex"0001ff70696e6700");
        vm.expectEmit(true, true, true, true, address(handler));
        emit MessageRequested(ID, SENDER, SOURCE, hex"0001ff70696e6700");
        vm.prank(SENDER);
        handler.sendMessage(ID, hex"0001ff70696e6700");
        vm.prank(SENDER);
        vm.expectRevert(abi.encodeWithSelector(MessageHandler.AlreadySent.selector, ID));
        handler.sendMessage(ID, hex"ff");
        assertFalse(handler.received(ID));
    }

    function testExactBytesAndIndependentReplayNamespaces() public {
        bytes memory payload = hex"ff000270696e6700";
        vm.expectEmit(true, true, false, true, address(handler));
        emit MessageHandled(SOURCE, ID, payload);
        vm.prank(QUEUE);
        handler.handleMessage(SOURCE, abi.encodePacked(ID, payload));
        assertTrue(handler.received(ID));
        assertEq(handler.payloadOf(ID), payload);
        // A new transport nonce still cannot redeliver an application ID.
        vm.prank(QUEUE);
        vm.expectRevert(abi.encodeWithSelector(MessageHandler.AlreadyReceived.selector, ID));
        handler.handleMessage(SOURCE, abi.encodePacked(ID, hex"01"));
        assertEq(handler.payloadOf(ID), payload);
        vm.prank(SENDER);
        handler.sendMessage(ID, payload);
    }

    function testPayloadBoundsAndZeroIds() public {
        vm.prank(QUEUE);
        vm.expectRevert(MessageHandler.InvalidPayload.selector);
        handler.handleMessage(SOURCE, new bytes(31));
        vm.prank(QUEUE);
        vm.expectRevert(MessageHandler.InvalidPayload.selector);
        handler.handleMessage(SOURCE, abi.encodePacked(bytes32(0), hex"01"));
        vm.prank(QUEUE);
        vm.expectRevert(MessageHandler.InvalidPayload.selector);
        handler.handleMessage(SOURCE, abi.encodePacked(ID, new bytes(1025)));
        vm.prank(SENDER);
        vm.expectRevert(MessageHandler.InvalidPayload.selector);
        handler.sendMessage(bytes32(0), hex"01");
        vm.prank(SENDER);
        vm.expectRevert(MessageHandler.InvalidPayload.selector);
        handler.sendMessage(ID, new bytes(1025));
        assertFalse(handler.received(ID));

        bytes memory maximum = new bytes(1024);
        maximum[0] = 0xff;
        maximum[1023] = 0x01;
        vm.prank(QUEUE);
        handler.handleMessage(SOURCE, abi.encodePacked(ID, maximum));
        assertEq(handler.payloadOf(ID), maximum);
        vm.prank(SENDER);
        handler.sendMessage(ID, maximum);
        bytes32 emptyId = bytes32(uint256(2));
        vm.prank(QUEUE);
        handler.handleMessage(SOURCE, abi.encodePacked(emptyId));
        assertTrue(handler.received(emptyId));
        assertEq(handler.payloadOf(emptyId), bytes(""));
        vm.prank(SENDER);
        handler.sendMessage(emptyId, bytes(""));
    }

    function testRequestIsNonpayable() public {
        vm.deal(SENDER, 1);
        vm.prank(SENDER);
        (bool success,) = address(handler).call{value: 1}(abi.encodeCall(handler.sendMessage, (ID, bytes(""))));
        assertFalse(success);
        vm.prank(SENDER);
        handler.sendMessage(ID, bytes(""));
    }
}
