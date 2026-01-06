// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {IBridgingPayment} from "./interfaces/IBridgingPayment.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @dev BridgingPayment smart contract is responsible for collecting bridging fees
 *      for transferring tokens from Ethereum to Vara. This smart contract enables
 *      setting fee amount, updating owner address that will collect the fee, and
 *      setting `ERC20Manager` smart contract that will handle bridging requests.
 */
contract BridgingPayment is IBridgingPayment, Ownable {
    address private immutable ERC20_MANAGER;

    uint256 public fee;

    /**
     * @dev Initializes the BridgingPayment contract with the ERC20Manager address, fee amount, and initial owner.
     * @param _erc20Manager The address of the ERC20Manager contract that will handle bridging requests.
     * @param _fee The initial fee amount required for bridging.
     * @param initialOwner The address that will receive the bridging fees.
     */
    constructor(address _erc20Manager, uint256 _fee, address initialOwner) Ownable(initialOwner) {
        ERC20_MANAGER = _erc20Manager;
        fee = _fee;
    }

    /**
     * @dev Returns the address of the ERC20Manager.
     */
    function erc20Manager() external view returns (address) {
        return ERC20_MANAGER;
    }

    /**
     * @dev Modifier to check if the caller is the ERC20Manager.
     */
    modifier onlyErc20Manager() {
        _onlyErc20Manager();
        _;
    }

    /**
     * @dev Internal function to check if the caller is the ERC20Manager.
     */
    function _onlyErc20Manager() internal view {
        if (msg.sender != ERC20_MANAGER) {
            revert OnlyErc20Manager();
        }
    }

    /**
     * @dev Sets the fee amount.
     * @param _fee The new fee amount.
     */
    function setFee(uint256 _fee) external onlyOwner {
        fee = _fee;
    }

    /**
     * @dev Pays the fee to the contract owner.
     */
    function payFee() external payable onlyErc20Manager {
        if (msg.value != fee) {
            revert IncorrectFeeAmount();
        }

        (bool success,) = owner().call{value: msg.value}("");
        if (!success) {
            revert PayFeeFailed();
        }

        emit FeePaid();
    }
}
