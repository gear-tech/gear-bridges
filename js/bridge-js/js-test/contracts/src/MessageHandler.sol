// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {IMessageHandler} from "ethereum/src/interfaces/IMessageHandler.sol";

contract MessageHandler is IMessageHandler {
    error NotQueue();
    error WrongSource();
    error NotSender();
    error InvalidPayload();
    error AlreadySent(bytes32 applicationId);
    error AlreadyReceived(bytes32 applicationId);

    address public immutable queue;
    bytes32 public immutable expectedVaraSource;
    address public immutable ethereumSender;

    mapping(bytes32 => bool) private _sent;
    mapping(bytes32 => bool) private _received;
    mapping(bytes32 => bytes) private _payloads;

    event MessageRequested(
        bytes32 indexed applicationId, address indexed sender, bytes32 indexed destination, bytes payload
    );
    event MessageHandled(bytes32 indexed source, bytes32 indexed applicationId, bytes payload);

    constructor(address queue_, bytes32 expectedVaraSource_, address ethereumSender_) {
        if (queue_ == address(0) || expectedVaraSource_ == bytes32(0) || ethereumSender_ == address(0)) {
            revert InvalidPayload();
        }
        queue = queue_;
        expectedVaraSource = expectedVaraSource_;
        ethereumSender = ethereumSender_;
    }

    function sendMessage(bytes32 applicationId, bytes calldata payload) external {
        if (msg.sender != ethereumSender) revert NotSender();
        if (applicationId == bytes32(0) || payload.length > 1024) revert InvalidPayload();
        if (_sent[applicationId]) revert AlreadySent(applicationId);
        _sent[applicationId] = true;
        emit MessageRequested(applicationId, msg.sender, expectedVaraSource, payload);
    }

    function handleMessage(bytes32 source, bytes calldata payload) external {
        if (msg.sender != queue) revert NotQueue();
        if (source != expectedVaraSource) revert WrongSource();
        if (payload.length < 32 || payload.length > 1056) revert InvalidPayload();
        bytes32 applicationId = bytes32(payload[:32]);
        if (applicationId == bytes32(0)) revert InvalidPayload();
        if (_received[applicationId]) revert AlreadyReceived(applicationId);
        _received[applicationId] = true;
        _payloads[applicationId] = payload[32:];
        emit MessageHandled(source, applicationId, payload[32:]);
    }

    function received(bytes32 applicationId) external view returns (bool) {
        return _received[applicationId];
    }

    function payloadOf(bytes32 applicationId) external view returns (bytes memory) {
        return _payloads[applicationId];
    }
}
