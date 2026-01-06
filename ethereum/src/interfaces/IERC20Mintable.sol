// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

/**
 * @dev Extension of {ERC20} that allows token holders to mint tokens.
 */
interface IERC20Mintable {
    /**
     * @dev Mints `amount` tokens to `to`.
     */
    function mint(address to, uint256 amount) external;
}
