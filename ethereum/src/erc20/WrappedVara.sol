// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC20Burnable} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Burnable.sol";
import {ERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Permit.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @dev Wrapped Vara (WVARA) is represents VARA on Ethereum as ERC20 token.
 *      VARA is also used for paying fees, staking and governance on Vara Network,
 *      while WVARA does all of the same things but on Ethereum.
 */
contract WrappedVara is ERC20, ERC20Burnable, Ownable, ERC20Permit {
    string private constant TOKEN_NAME = "Wrapped Vara";
    string private constant TOKEN_SYMBOL = "WVARA";

    /**
     * @dev Initializes the WrappedVara contract with the token name and symbol.
     * @param initialOwner The address of the initial owner of the contract.
     */
    constructor(address initialOwner) ERC20(TOKEN_NAME, TOKEN_SYMBOL) Ownable(initialOwner) ERC20Permit(TOKEN_NAME) {}

    /**
     * @dev Returns the number of decimals used to get its user representation.
     *      Also see documentation about decimals:
     *      - https://wiki.vara.network/docs/staking/validator-faqs#what-is-the-precision-of-the-vara-token
     */
    function decimals() public view virtual override returns (uint8) {
        return 12;
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
