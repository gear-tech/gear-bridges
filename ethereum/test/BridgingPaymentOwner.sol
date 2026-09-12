// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {ERC20Manager} from "src/ERC20Manager.sol";

contract BridgingPaymentOwner {
    ERC20Manager private immutable ERC20_MANAGER;

    constructor(ERC20Manager _erc20Manager) {
        ERC20_MANAGER = _erc20Manager;
    }

    function createBridgingPayment(uint256 fee) external returns (address) {
        return ERC20_MANAGER.createBridgingPayment(fee);
    }
}
