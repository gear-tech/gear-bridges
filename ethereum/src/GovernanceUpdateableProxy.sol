pragma solidity ^0.8.24;

import {IMessageQueueReceiver} from "./interfaces/IMessageQueue.sol";

contract GovernanceUpdateableProxy is IMessageQueueReceiver {
    error ProxyDeniedAdminAccess();
    error InvalidDiscriminator(uint8 discriminator);
    error InvalidPayloadLength();

    address implementation;
    bytes32 governance;
    address messageQueue;

    constructor(
        address _implementation,
        address _messageQueue,
        bytes32 _governance
    ) payable {
        implementation = _implementation;
        messageQueue = _messageQueue;
        governance = _governance;
    }

    /** @dev Accept request from MessageQueue. Based on the first byte of the payload
     * make the decision what to do.
     *
     * If first byte = `0x00` then delegate call to the implementation (dropping first byte).
     * If first byte = `0x01` then update implementation address(can be called only by current governance).
     * If first byte = `0x02` then change governance(can be called only by current governance).
     *
     * @param sender sender of message on the gear side.
     * @param payload payload of the message.
     */
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

            address new_implementation = address(bytes20(payload[1:]));

            if (msg.sender == messageQueue && sender == governance) {
                implementation = new_implementation;
                return true;
            } else {
                revert ProxyDeniedAdminAccess();
            }
        } else if (discriminator == 0x02) {
            // Change governance.

            if (payload.length != 1 + 32) {
                revert InvalidPayloadLength();
            }

            bytes32 new_governance = bytes32(payload[1:]);

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
        (bool success, bytes memory data) = implementation.delegatecall(
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

    function getImplementation() public view returns (address) {
        return implementation;
    }

    function getGovernance() public view returns (bytes32) {
        return governance;
    }
}
