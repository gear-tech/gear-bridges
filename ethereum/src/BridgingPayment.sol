pragma solidity ^0.8.24;

import {Proxy} from "@openzeppelin/contracts/proxy/Proxy.sol";
import "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";

contract BridgingPayment is Proxy {
    error ProxyDeniedAdminAccess();
    error NotEnoughFunds();

    uint256 fee;

    constructor(uint256 _fee) payable {
        fee = _fee;

        ERC1967Utils.changeAdmin(msg.sender);
    }

    function _delegate(address impl) internal override {
        (bool feeTransferSuccess, ) = getAdmin().call{value: fee}("");
        if (!feeTransferSuccess) {
            revert NotEnoughFunds();
        }

        super._delegate(impl);
    }

    function setFee(uint256 newFee) public {
        if (msg.sender != ERC1967Utils.getAdmin()) {
            revert ProxyDeniedAdminAccess();
        } else {
            fee = newFee;
        }
    }

    function setAdmin(address newAdmin) public {
        if (msg.sender != ERC1967Utils.getAdmin()) {
            revert ProxyDeniedAdminAccess();
        } else {
            ERC1967Utils.changeAdmin(newAdmin);
        }
    }

    receive() external payable {
        _delegate(_implementation());
    }

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

    function getAdmin() public view returns (address) {
        return ERC1967Utils.getAdmin();
    }

    function getFee() public view returns (uint256) {
        return fee;
    }
}
