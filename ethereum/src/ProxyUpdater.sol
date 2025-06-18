// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IMessageQueueReceiver} from "./interfaces/IMessageQueueReceiver.sol";
import {ProxyContract} from "./ProxyContract.sol";

contract ProxyUpdater is IMessageQueueReceiver {
    error NotAuthorized();
    error NotGovernance();
    error BadArguments();
    error InvalidDiscriminant();

    ProxyContract proxy;
    bytes32 governance;

    address immutable MESSAGE_QUEUE;

    constructor(ProxyContract _proxy, bytes32 _governance, address message_queue) {
        proxy = _proxy;
        governance = _governance;
        MESSAGE_QUEUE = message_queue;
    }

    /**
     * @dev Accept request from MessageQueue. Based on the first byte of the payload
     * make the decision what to do.
     *
     * If first byte = `0x00` then update implementation of underlying proxy.
     * If first byte = `0x01` then change admin of the underlying proxy.
     * If first byte = `0x02` then change governance.
     *
     * @param sender sender of message on the gear side.
     * @param payload payload of the message.
     */
    function processVaraMessage(bytes32 sender, bytes calldata payload) external returns (bool) {
        if (msg.sender != MESSAGE_QUEUE) {
            revert NotAuthorized();
        }
        if (sender != governance) {
            revert NotGovernance();
        }

        uint8 discriminant = uint8(payload[0]);

        if (discriminant == 0x00) {
            if (payload.length < 1 + 20) {
                revert BadArguments();
            }

            address new_implementation = address(bytes20(payload[1:21]));
            bytes calldata data = payload[21:];

            proxy.upgradeToAndCall(new_implementation, data);
        } else if (discriminant == 0x01) {
            if (payload.length != 1 + 20) {
                revert BadArguments();
            }

            address new_admin = address(bytes20(payload[1:]));

            proxy.changeProxyAdmin(new_admin);
        } else if (discriminant == 0x02) {
            if (payload.length != 1 + 32) {
                revert BadArguments();
            }

            governance = bytes32(payload[1:]);
        } else {
            revert InvalidDiscriminant();
        }

        return true;
    }

    function getGovernance() external view returns (bytes32) {
        return governance;
    }
}
