// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IMessageHandler} from "./IMessageHandler.sol";

/**
 * @dev Governance constants.
 */
library GovernanceConstants {
    /**
     * @dev Change governance message discriminant.
     */
    uint256 internal constant CHANGE_GOVERNANCE = 0x00;
    /**
     * @dev Pause proxy message discriminant.
     */
    uint256 internal constant PAUSE_PROXY = 0x01;
    /**
     * @dev Unpause proxy message discriminant.
     */
    uint256 internal constant UNPAUSE_PROXY = 0x02;
    /**
     * @dev Upgrade proxy message discriminant.
     */
    uint256 internal constant UPGRADE_PROXY = 0x03;

    /**
     * @dev `uint8 discriminant` size.
     */
    uint256 internal constant DISCRIMINANT_SIZE = 1;
    /**
     * @dev `bytes32 newGovernance` size.
     */
    uint256 internal constant NEW_GOVERNANCE_SIZE = 32;
    /**
     * @dev `address proxy` size.
     */
    uint256 internal constant PROXY_ADDRESS_SIZE = 20;
    /**
     * @dev `address newImplementation` size.
     */
    uint256 internal constant NEW_IMPLEMENTATION_SIZE = 20;

    /**
     * @dev `DISCRIMINANT_SIZE` offset.
     */
    uint256 internal constant OFFSET1 = 1;
    /**
     * @dev `DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE` offset.
     */
    uint256 internal constant OFFSET2 = 21;
    /**
     * @dev `DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE + NEW_IMPLEMENTATION_SIZE` offset.
     */
    uint256 internal constant OFFSET3 = 41;

    /**
     * @dev `DISCRIMINANT_SIZE + NEW_GOVERNANCE_SIZE` size.
     */
    uint256 internal constant CHANGE_GOVERNANCE_SIZE = 33;
    /**
     * @dev `DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE` size.
     */
    uint256 internal constant PAUSE_UNPAUSE_PROXY_SIZE = 21;
    /**
     * @dev `DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE + NEW_IMPLEMENTATION_SIZE` size.
     */
    uint256 internal constant UPGRADE_PROXY_SIZE = 41;
}

/**
 * @dev Interface for the Governance contract.
 */
interface IGovernance is IMessageHandler {
    /**
     * @dev Error thrown when the sender is not the message queue.
     */
    error InvalidSender();

    /**
     * @dev Error thrown when the source is not the governance.
     */
    error InvalidSource();

    /**
     * @dev Error thrown when the payload is invalid.
     */
    error InvalidPayload();

    /**
     * @dev Emitted when the governance address is changed.
     * @param previousGovernance The previous governance address.
     * @param newGovernance The new governance address.
     */
    event GovernanceChanged(bytes32 indexed previousGovernance, bytes32 indexed newGovernance);

    /**
     * @dev Returns the governance address.
     * @return governance The governance address.
     */
    function governance() external view returns (bytes32);

    /**
     * @dev Returns the message queue address.
     * @return messageQueue The message queue address.
     */
    function messageQueue() external view returns (address);
}

/**
 * @dev Type representing payload of the message that changes governance address.
 */
struct ChangeGovernanceMessage {
    bytes32 newGovernance;
}

/**
 * @dev Type representing payload of the message that pauses proxy.
 */
struct PauseProxyMessage {
    address proxy;
}

/**
 * @dev Type representing payload of the message that unpauses proxy.
 */
struct UnpauseProxyMessage {
    address proxy;
}

/**
 * @dev Type representing payload of the message that upgrades proxy.
 */
struct UpgradeProxyMessage {
    address proxy;
    address newImplementation;
    bytes data;
}

/**
 * @dev Library for packing `Governance` messages into a binary format.
 */
library GovernancePacker {
    /**
     * @dev Packs `ChangeGovernanceMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(ChangeGovernanceMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(uint8(GovernanceConstants.CHANGE_GOVERNANCE), message.newGovernance);
    }

    /**
     * @dev Packs `PauseProxyMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(PauseProxyMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(uint8(GovernanceConstants.PAUSE_PROXY), message.proxy);
    }

    /**
     * @dev Packs `UnpauseProxyMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(UnpauseProxyMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(uint8(GovernanceConstants.UNPAUSE_PROXY), message.proxy);
    }

    /**
     * @dev Packs `UpgradeProxyMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(UpgradeProxyMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(
            uint8(GovernanceConstants.UPGRADE_PROXY), message.proxy, message.newImplementation, message.data
        );
    }
}
