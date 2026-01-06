// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC20Burnable} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Burnable.sol";
import {ERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Permit.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @dev USD Coin (USDC) is represents USDC on Ethereum as ERC20 token.
 *      USDC is stablecoin that is pegged to US dollar.
 *
 *      Based on USDC:
 *      - https://etherscan.io/address/0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48#code
 *
 *      This smart contract tries to be as similar as possible to Ethereum mainnet USDC.
 */
contract CircleToken is ERC20, ERC20Burnable, Ownable, ERC20Permit {
    string private constant TOKEN_NAME = "USD Coin";
    string private constant TOKEN_SYMBOL = "USDC";

    /**
     * @dev Initializes the CircleToken contract with the token name and symbol.
     * @param initialOwner The address of the initial owner of the contract.
     */
    constructor(address initialOwner) ERC20(TOKEN_NAME, TOKEN_SYMBOL) Ownable(initialOwner) ERC20Permit(TOKEN_NAME) {}

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
