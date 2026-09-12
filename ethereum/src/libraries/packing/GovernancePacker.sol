// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {GovernanceConstants} from "src/GovernanceConstants.sol";

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
        // casting to 'uint8' is safe because [explain why]
        // forge-lint: disable-next-item(unsafe-typecast)
        return abi.encodePacked(uint8(GovernanceConstants.CHANGE_GOVERNANCE), message.newGovernance);
    }

    /**
     * @dev Packs `PauseProxyMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(PauseProxyMessage memory message) internal pure returns (bytes memory) {
        // casting to 'uint8' is safe because [explain why]
        // forge-lint: disable-next-item(unsafe-typecast)
        return abi.encodePacked(uint8(GovernanceConstants.PAUSE_PROXY), message.proxy);
    }

    /**
     * @dev Packs `UnpauseProxyMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(UnpauseProxyMessage memory message) internal pure returns (bytes memory) {
        // casting to 'uint8' is safe because [explain why]
        // forge-lint: disable-next-item(unsafe-typecast)
        return abi.encodePacked(uint8(GovernanceConstants.UNPAUSE_PROXY), message.proxy);
    }

    /**
     * @dev Packs `UpgradeProxyMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(UpgradeProxyMessage memory message) internal pure returns (bytes memory) {
        // casting to 'uint8' is safe because [explain why]
        // forge-lint: disable-next-item(unsafe-typecast)
        return abi.encodePacked(
            uint8(GovernanceConstants.UPGRADE_PROXY), message.proxy, message.newImplementation, message.data
        );
    }
}
