// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/**
 * @dev Wrapped Ether (WETH) is represents Ether on Ethereum as ERC20 token.
 *      Based on WETH9:
 *      - https://etherscan.io/address/0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2#code
 */
contract WrappedEther is ERC20 {
    /**
     * @dev Emitted when `value` Ether is wrapped into tokens for account `to`.
     *
     * This event indicates that `to` deposited Ether and received an equivalent amount
     * of wrapped tokens.
     *
     * Note that `value` may be zero.
     */
    event Deposit(address indexed to, uint256 value);

    /**
     * @dev Emitted when `value` tokens are unwrapped into Ether by account `from`.
     *
     * This event indicates that `from` burned wrapped tokens and received an equivalent
     * amount of Ether.
     *
     * Note that `value` may be zero.
     */
    event Withdrawal(address indexed from, uint256 value);

    string private constant TOKEN_NAME = "Wrapped Ether";
    string private constant TOKEN_SYMBOL = "WETH";

    /**
     * @dev Initializes the WrappedEther contract with the token name and symbol.
     */
    constructor() ERC20(TOKEN_NAME, TOKEN_SYMBOL) {}

    /**
     * @dev Deposits Ether into the contract and mints an equivalent amount of wrapped tokens to the caller.
     *
     * Emits a {Deposit} event indicating the recipient and the amount deposited.
     */
    function deposit() public payable virtual {
        address to = _msgSender();
        uint256 value = msg.value;
        _mint(to, value);

        emit Deposit(to, value);
    }

    /**
     * @dev Burns a specified amount of wrapped tokens from the caller and withdraws the equivalent amount of Ether.
     *
     * Emits a {Withdrawal} event indicating the sender and the amount withdrawn.
     *
     * Requirements:
     * - The caller must have at least `value` tokens.
     * - The contract must successfully send `value` Ether to the caller.
     *
     * Note:
     * - Uses a low-level call to send Ether, which forwards all available gas.
     * - Reverts if the Ether transfer fails.
     */
    function withdraw(uint256 value) public virtual {
        address from = _msgSender();
        _burn(from, value);

        (bool success,) = from.call{value: value}("");
        require(success);

        emit Withdrawal(from, value);
    }

    /**
     * @dev Fallback function to receive Ether. Automatically deposits received Ether
     * and mints wrapped tokens to the sender.
     *
     * Emits a {Deposit} event as part of the `deposit()` call.
     */
    receive() external payable virtual {
        deposit();
    }
}
