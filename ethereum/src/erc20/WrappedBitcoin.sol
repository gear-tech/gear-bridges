// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC20Burnable} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Burnable.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @dev Wrapped Bitcoin (WBTC) is represents Bitcoin on Ethereum as ERC20 token.
 *
 *      Based on WBTC:
 *      - https://etherscan.io/address/0x2260fac5e5542a773aa44fbcfedf7c193bc2c599#code
 *
 *      This smart contract tries to be as similar as possible to Ethereum mainnet WBTC.
 */
contract WrappedBitcoin is ERC20, ERC20Burnable, Ownable {
    string private constant TOKEN_NAME = "Wrapped BTC";
    string private constant TOKEN_SYMBOL = "WBTC";

    /**
     * @dev Initializes the WrappedBitcoin contract with the token name and symbol.
     * @param initialOwner The address of the initial owner of the contract.
     */
    constructor(address initialOwner) ERC20(TOKEN_NAME, TOKEN_SYMBOL) Ownable(initialOwner) {}

    /**
     * @dev Returns the number of decimals used to get its user representation.
     */
    function decimals() public view virtual override returns (uint8) {
        return 8;
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
