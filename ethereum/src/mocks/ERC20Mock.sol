// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {Context} from "@openzeppelin/contracts/utils/Context.sol";

contract ERC20Mock is Context, ERC20 {
    constructor(string memory _symbol) ERC20(_symbol, _symbol) {
        _mint(_msgSender(), type(uint256).max);
    }

    function decimals() public pure override returns (uint8) {
        return 18;
    }
}
