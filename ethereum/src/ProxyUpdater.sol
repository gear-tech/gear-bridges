// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IMessageQueueReceiver} from "./interfaces/IMessageQueueReceiver.sol";
import {ProxyContract} from "./ProxyContract.sol";

contract ProxyUpdater is IMessageQueueReceiver {
    error NotAuthorized();
    error NotGovernance();
    error BadArguments();
    error InvalidDiscriminant();

    address immutable MESSAGE_QUEUE;

    /**
     * @dev Size of update implementation message.
     *
     *      ```solidity
     *      struct UpdateImplementationMessage {
     *          uint8 discriminant; // 1 byte
     *          address newImplementation; // 20 bytes
     *      }
     */
    uint256 private constant UPDATE_IMPLEMENTATION_SIZE = 21; //1 + 20

    /**
     * @dev Size of schange admin message.
     *
     *      ```solidity
     *      struct ChangeAdminMessage {
     *          uint8 discriminant; // 1 byte
     *          address newAdmin; // 20 bytes
     *      }
     */
    uint256 private constant CHANGE_ADMIN_SIZE = 21; //1 + 20

    /**
     * @dev Size of change governance message.
     *
     *      ```solidity
     *      struct ChangeGovernanceMessage {
     *          uint8 discriminant; // 1 byte
     *          bytes32 newGovernance; // 32 bytes
     *      }
     */
    uint256 private constant CHANGE_GOVERNANCE_SIZE = 33; //1 + 32

    ProxyContract proxy;
    bytes32 governance;

    constructor(ProxyContract _proxy, bytes32 _governance, address messageQueue) {
        proxy = _proxy;
        governance = _governance;
        MESSAGE_QUEUE = messageQueue;
    }

    /**
     * @dev Accept request from MessageQueue. Based on the first byte of the payload
     *      make the decision what to do.
     *
     *      If first byte = `0x00` then update implementation of underlying proxy.
     *      If first byte = `0x01` then change admin of the underlying proxy.
     *      If first byte = `0x02` then change governance.
     *
     * @param sender sender of message on the gear side.
     * @param payload payload of the message.
     */
    function processVaraMessage(bytes32 sender, bytes calldata payload) external {
        if (msg.sender != MESSAGE_QUEUE) {
            revert NotAuthorized();
        }
        if (sender != governance) {
            revert NotGovernance();
        }

        uint8 discriminant = uint8(payload[0]);

        if (discriminant == 0x00) {
            if (payload.length < UPDATE_IMPLEMENTATION_SIZE) {
                revert BadArguments();
            }

            address newImplementation = address(bytes20(payload[1:21]));
            bytes calldata data = payload[21:];

            proxy.upgradeToAndCall(newImplementation, data);
        } else if (discriminant == 0x01) {
            if (payload.length != CHANGE_ADMIN_SIZE) {
                revert BadArguments();
            }

            address newAdmin = address(bytes20(payload[1:]));

            proxy.changeProxyAdmin(newAdmin);
        } else if (discriminant == 0x02) {
            if (payload.length != CHANGE_GOVERNANCE_SIZE) {
                revert BadArguments();
            }

            bytes32 newGovernance = bytes32(payload[1:]);

            governance = newGovernance;
        } else {
            revert InvalidDiscriminant();
        }
    }

    function getGovernance() external view returns (bytes32) {
        return governance;
    }
}
