// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {LibString} from "src/libraries/LibString.sol";

/**
 * @dev Type representing payload of the message that `ERC20Manager` will accept
 *      from `gear-programs/vft-manager` program.
 *      Also see `gear-programs/vft-manager/app/src/services/request_bridging/bridge_builtin_operations.rs`.
 */
struct TransferMessage {
    bytes32 sender;
    address receiver;
    address token;
    uint256 amount;
}

/**
 * @dev Type representing payload of the message that adds VFT manager to list of registered VFT managers.
 */
struct AddVftManagerMessage {
    bytes32 vftManager;
}

/**
 * @dev Type representing payload of the message that registers Ethereum token.
 */
struct RegisterEthereumTokenMessage {
    address token;
}

/**
 * @dev Type representing payload of the message that registers Gear token.
 */
struct RegisterGearTokenMessage {
    string tokenName;
    string tokenSymbol;
    uint8 tokenDecimals;
}

/**
 * @dev Library for packing `ERC20Manager` messages into a binary format.
 */
library ERC20ManagerPacker {
    /**
     * @dev Packs `TransferMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(TransferMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(message.sender, message.receiver, message.token, message.amount);
    }

    /// forge-lint: disable-next-item(internal-function-used-once)
    /**
     * @dev Packs `AddVftManagerMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(AddVftManagerMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(uint8(0x00), message.vftManager);
    }

    /// forge-lint: disable-next-item(internal-function-used-once)
    /**
     * @dev Packs `RegisterEthereumTokenMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(RegisterEthereumTokenMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(uint8(IERC20Manager.TokenType.Ethereum), message.token);
    }

    /// forge-lint: disable-next-item(internal-function-used-once)
    /**
     * @dev Packs `RegisterGearTokenMessage` into a binary format.
     * @param message Message to pack.
     * @return packed Packed message.
     */
    function pack(RegisterGearTokenMessage memory message) internal pure returns (bytes memory) {
        return abi.encodePacked(
            uint8(IERC20Manager.TokenType.Gear),
            LibString.packOne(message.tokenName),
            LibString.packOne(message.tokenSymbol),
            message.tokenDecimals
        );
    }
}
