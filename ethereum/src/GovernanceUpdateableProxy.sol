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

            return _delegate(sender, payload[1:]);
        } else if (discriminator == 0x01) {
            // Change implementation.

            if (payload.length != 1 + 20) {
                revert InvalidPayloadLength();
            }

            address new_impl = abi.decode(payload[1:21], (address));

            if (msg.sender == messageQueue && sender == governance) {
                impl = new_impl;
                return true;
            } else {
                revert ProxyDeniedAdminAccess();
            }
        } else if (discriminator == 0x02) {
            // Change governance.

            if (payload.length != 1 + 32) {
                revert InvalidPayloadLength();
            }

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

    function _delegate(
        bytes32 sender,
        bytes calldata payload
    ) internal returns (bool) {
        (bool success, bytes memory data) = impl.delegatecall(
            abi.encodeWithSignature(
                "processVaraMessage(bytes32,bytes)",
                sender,
                payload
            )
        );

        if (!success) {
            assembly {
                let size := mload(data)
                revert(add(32, data), size)
            }
        }

        return abi.decode(data, (bool));
    }

    function implementation() public view returns (address) {
        return impl;
    }
}
