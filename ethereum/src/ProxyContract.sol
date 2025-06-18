// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Proxy} from "@openzeppelin/contracts/proxy/Proxy.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";

contract ProxyContract is Proxy {
    error ProxyDeniedAdminAccess();

    /**
     * @dev Initializes an upgradeable proxy managed by an instance of a {ProxyAdmin} with an
     * `initialOwner`,backed by the implementation at `_logic`, and optionally initialized with
     * `_data` as explained in {ERC1967Proxy-constructor}.
     */
    constructor() payable {
        ERC1967Utils.changeAdmin(msg.sender);
    }

    /**
     * @dev Upgrade the implementation of the proxy. See {ERC1967Utils-upgradeToAndCall}.
     *
     * Requirements:
     *
     * - If `data` is empty, `msg.value` must be zero.
     */
    function upgradeToAndCall(
        address newImplementation,
        bytes calldata data
    ) public {
        if (msg.sender != ERC1967Utils.getAdmin()) {
            revert ProxyDeniedAdminAccess();
        } else {
            ERC1967Utils.upgradeToAndCall(newImplementation, data);
        }
    }

    function changeProxyAdmin(address newAdmin) public {
        if (msg.sender != ERC1967Utils.getAdmin()) {
            revert ProxyDeniedAdminAccess();
        } else {
            ERC1967Utils.changeAdmin(newAdmin);
        }
    }

    /**
     * @dev Returns the current implementation address.
     *
     * TIP: To get this value clients can read directly from the storage slot shown below (specified by ERC-1967) using
     * the https://eth.wiki/json-rpc/API#eth_getstorageat[`eth_getStorageAt`] RPC call.
     * `0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc`
     */
    function _implementation() internal view virtual override returns (address) {
        return ERC1967Utils.getImplementation();
    }
}
