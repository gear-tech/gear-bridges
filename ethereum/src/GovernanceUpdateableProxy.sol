pragma solidity ^0.8.24;

import {Proxy} from "@openzeppelin/contracts/proxy/Proxy.sol";
import "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";

import {IMessageQueueReceiver} from "./interfaces/IMessageQueue.sol";

contract GovernanceUpdateableProxy is Proxy, IMessageQueueReceiver {
    error ProxyDeniedAdminAccess();
    error InvalidDiscriminator(uint8 discriminator);

    constructor(address messageQueue) payable {
        ERC1967Utils.changeAdmin(messageQueue);
    }

    function processVaraMessage(
        bytes32 sender,
        bytes calldata payload
    ) external returns (bool) {
        uint8 discriminator = uint8(payload[0]);
        if (discriminator == 0x00) {
            // TODO: Delegate call trimming the first byte

            return true;
        } else if (discriminator == 0x01) {
            address new_implementation = abi.decode(payload[1:21], (address));
            bytes calldata data = payload[21:];

            if (msg.sender != ERC1967Utils.getAdmin()) {
                revert ProxyDeniedAdminAccess();
            } else {
                ERC1967Utils.upgradeToAndCall(new_implementation, data);
            }

            return true;
        } else {
            revert InvalidDiscriminator(discriminator);
        }
    }

    receive() external payable {}

    /**
     * @dev Returns the current implementation address.
     *
     * TIP: To get this value clients can read directly from the storage slot shown below (specified by ERC-1967) using
     * the https://eth.wiki/json-rpc/API#eth_getstorageat[`eth_getStorageAt`] RPC call.
     * `0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc`
     */
    function _implementation()
        internal
        view
        virtual
        override
        returns (address)
    {
        return ERC1967Utils.getImplementation();
    }

    function implementation() public view returns (address) {
        return _implementation();
    }
}
