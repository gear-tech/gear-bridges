// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC20Burnable} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Burnable.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @dev Tether USD (USDT) is represents USDT on Ethereum as ERC20 token.
 *      USDT is stablecoin that is pegged to US dollar.
 *
 *      Based on USDT:
 *      - https://etherscan.io/address/0xdac17f958d2ee523a2206206994597c13d831ec7#code
 *
 *      This smart contract tries to be as similar as possible to Ethereum mainnet USDT.
 */
contract TetherToken is ERC20, ERC20Burnable, Ownable {
    string private constant TOKEN_NAME = "Tether USD";
    string private constant TOKEN_SYMBOL = "USDT";

    /**
     * @dev Initializes the TetherToken contract with the token name and symbol.
     * @param initialOwner The address of the initial owner of the contract.
     */
    constructor(address initialOwner) ERC20(TOKEN_NAME, TOKEN_SYMBOL) Ownable(initialOwner) {}

    /**
     * @dev Returns the number of decimals used to get its user representation.
     */
    function decimals() public view virtual override returns (uint8) {
        return 6;
    }

    /**
     * @dev Mints `amount` tokens to `to`.
     * @param to The address to mint tokens to.
     * @param amount The amount of tokens to mint.
     */
    function mint(address to, uint256 amount) public onlyOwner {
        _mint(to, amount);
    }
}
