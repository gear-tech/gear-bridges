// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Interface for the BridgingPayment contract.
 */
interface IBridgingPayment {
    /**
     * @dev The caller is not the ERC20Manager.
     */
    error OnlyErc20Manager();

    /**
     * @dev The fee amount is incorrect.
     */
    error IncorrectFeeAmount();

    /**
     * @dev The fee payment failed (transfer failed)
     */
    error PayFeeFailed();

    /**
     * @dev Emitted when fee is paid.
     */
    event FeePaid();

    /**
     * @dev Returns the address of the ERC20Manager.
     */
    function erc20Manager() external view returns (address);

    /**
     * @dev Returns the fee amount.
     */
    function fee() external view returns (uint256);

    /**
     * @dev Sets the fee amount.
     */
    function setFee(uint256 fee) external;

    /**
     * @dev Pays the fee to the contract owner.
     */
    function payFee() external payable;
}
