pragma solidity ^0.8.24;

import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";


contract ProxyContract is ERC1967Proxy {
    error ProxyDeniedAdminAccess();

    /**
     * @dev Initializes an upgradeable proxy managed by an instance of a {ProxyAdmin} with an `initialOwner`,
     * backed by the implementation at `_logic`, and optionally initialized with `_data` as explained in
     * {ERC1967Proxy-constructor}.
     */
    constructor(address _logic, bytes memory _data) payable ERC1967Proxy(_logic, _data) {
        ERC1967Utils.changeAdmin(msg.sender);
    }

    /**
     * @dev Returns the admin of this proxy.
     */
    function _proxyAdmin() internal view virtual returns (address) {
        return ERC1967Utils.getAdmin();
    }

    /**
     * @dev If caller is the admin process the call internally, otherwise transparently fallback to the proxy behavior.
     */



    fallback() override external payable  {
        super._fallback();
    }

    receive() external payable  {
    }


    /**
     * @dev Upgrade the implementation of the proxy. See {ERC1967Utils-upgradeToAndCall}.
     *
     * Requirements:
     *
     * - If `data` is empty, `msg.value` must be zero.
     */

    function upgradeToAndCall(address newImplementation, bytes calldata data) public {
        if (msg.sender != _proxyAdmin()) {
            revert ProxyDeniedAdminAccess();
        }else{
            _dispatchUpgradeToAndCall(newImplementation, data);
        }    
    }

    function _dispatchUpgradeToAndCall(address newImplementation, bytes calldata data) private {
        ERC1967Utils.upgradeToAndCall(newImplementation, data);
    }

    function changeProxyAdmin(address newAdmin) public {
        if (msg.sender != _proxyAdmin()) {
            revert ProxyDeniedAdminAccess();
        }else{
            ERC1967Utils.changeAdmin(newAdmin);
        }    

    }

    function implementation() public view returns(address) {
        return _implementation();
    }

    function proxyAdmin() public view returns(address) {
       return  _proxyAdmin();
    }
    


}