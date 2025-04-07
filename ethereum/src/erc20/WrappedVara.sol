pragma solidity ^0.8.24;

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
