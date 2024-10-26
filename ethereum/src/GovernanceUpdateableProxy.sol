pragma solidity ^0.8.24;

import {IMessageQueueReceiver} from "./interfaces/IMessageQueue.sol";

contract GovernanceUpdateableProxy is IMessageQueueReceiver {
    error ProxyDeniedAdminAccess();
    error InvalidDiscriminator(uint8 discriminator);
    error InvalidPayloadLength();

    address impl;
    bytes32 governance;
    address messageQueue;

    constructor(address _messageQueue, bytes32 _governance) payable {
        messageQueue = _messageQueue;
        governance = _governance;
    }

    function processVaraMessage(
        bytes32 sender,
        bytes calldata payload
    ) external returns (bool) {
        uint8 discriminator = uint8(payload[0]);
        if (discriminator == 0x00) {
            // Delegate call.

            // TODO: Delegate call trimming the first byte

            return true;
        } else if (discriminator == 0x01) {
            // Change implementation.

            require(payload.length == 1 + 20, InvalidPayloadLength());

            address new_impl = abi.decode(payload[1:21], (address));

            if (msg.sender == messageQueue && sender == governance) {
                impl = new_impl;
                return true;
            } else {
                revert ProxyDeniedAdminAccess();
            }
        } else if (discriminator == 0x02) {
            // Change governance.

            require(payload.length == 1 + 32, InvalidPayloadLength());

            bytes32 new_governance = abi.decode(payload[1:33], (bytes32));

            if (msg.sender == messageQueue && sender == governance) {
                governance = new_governance;
                return true;
            } else {
                revert ProxyDeniedAdminAccess();
            }
        } else {
            revert InvalidDiscriminator(discriminator);
        }
    }

    function implementation() public view returns (address) {
        return impl;
    }
}
