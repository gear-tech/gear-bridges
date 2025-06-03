// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {ERC20GearSupply} from "./ERC20GearSupply.sol";

contract WrappedVara is ERC20GearSupply {
    constructor(
        address owner,
        string memory name,
        string memory symbol
    ) ERC20GearSupply(owner, name, symbol) {}

    function decimals() public pure override returns (uint8) {
        return 12;
    }
}
