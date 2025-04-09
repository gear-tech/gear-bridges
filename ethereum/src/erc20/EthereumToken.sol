pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract EthereumToken is ERC20 {
    constructor(
        string memory _name,
        string memory _symbol
    ) ERC20(_name, _symbol) {}

    function decimals() public pure override returns (uint8) {
        return 18;
    }

    /** @dev Convert native Ethereum into the `EthereumToken` ERC-20 token.
     *
     *  The conversion will be done in 1:1 ratio.
     */
    function tokenize() public payable {
        _mint(msg.sender, msg.value);
    }

    /** @dev Convert `EthereumToken` ERC-20 token into native Ethereum.
     *
     *  The conversion will be done in 1:1 ratio.
     *
     *  @param amount - Amount of the `EthereumToken` to be exchanged to a native token.
     */
    function release(uint256 amount) public {
        _burn(msg.sender, amount);
        payable(msg.sender).transfer(amount);
    }
}
