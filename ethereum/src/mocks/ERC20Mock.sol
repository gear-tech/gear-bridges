pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {Context} from "@openzeppelin/contracts/utils/Context.sol";

contract ERC20Mock is Context, ERC20 {
    constructor(string memory _symbol) ERC20(_symbol, _symbol) {
        _mint(_msgSender(), 1_000_000_000 * (10 ** 18));
    }
}
