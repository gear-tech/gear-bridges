pragma solidity ^0.8.24;

import {IMessageQueueReceiver} from "./interfaces/IMessageQueue.sol";
import {ProxyContract} from "./ProxyContract.sol";

contract ProxyUpdater is IMessageQueueReceiver {
    error NotAuthorized();
    error NotGovernance();
    error BadArguments();
    error WrongDiscriminator();

    ProxyContract proxy;
    bytes32 governance;
    address immutable MESSAGE_QUEUE_ADDRESS;

    constructor(
        address payable _proxy,
        bytes32 _governance,
        address message_queue
    ) payable {
        proxy = ProxyContract(_proxy);
        governance = _governance;
        MESSAGE_QUEUE_ADDRESS = message_queue;
    }

    function processVaraMessage(
        bytes32 sender,
        bytes calldata payload
    ) external returns (bool) {
        if (msg.sender != MESSAGE_QUEUE_ADDRESS) {
            revert NotAuthorized();
        }
        if (sender != governance) {
            revert NotGovernance();
        }

        uint8 discriminator = abi.decode(payload[:1], (uint8));

        if (discriminator == 0x00) {
            if (payload.length < 1 + 20) {
                revert BadArguments();
            }

            address new_implementation = abi.decode(payload[1:21], (address));
            bytes calldata data = payload[21:];

            proxy.upgradeToAndCall(new_implementation, data);
        } else if (discriminator == 0x01) {
            if (payload.length != 1 + 20) {
                revert BadArguments();
            }

            address new_admin = abi.decode(payload[1:], (address));

            proxy.changeProxyAdmin(new_admin);
        } else if (discriminator == 0x02) {
            if (payload.length != 1 + 32) {
                revert BadArguments();
            }

            governance = abi.decode(payload[1:], (bytes32));
        } else {
            revert WrongDiscriminator();
        }

        return true;
    }
}
