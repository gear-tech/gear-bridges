// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC20Burnable} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Burnable.sol";
import {ERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Permit.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

contract ERC20GearSupply is ERC20, ERC20Burnable, Ownable, ERC20Permit {
    uint8 private _decimals;

    /**
     * @dev Initializes the ERC20GearSupply contract with the token name and symbol.
     * @param initialOwner The address of the initial owner of the contract.
     * @param tokenName The name of the token.
     * @param tokenSymbol The symbol of the token.
     * @param tokenDecimals The number of decimals used to get its user representation.
     */
    constructor(address initialOwner, string memory tokenName, string memory tokenSymbol, uint8 tokenDecimals)
        ERC20(tokenName, tokenSymbol)
        Ownable(initialOwner)
        ERC20Permit(tokenName)
    {
        _decimals = tokenDecimals;
    }

    /**
     * @dev Returns the number of decimals used to get its user representation.
     */
    function decimals() public view virtual override returns (uint8) {
        return _decimals;
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
