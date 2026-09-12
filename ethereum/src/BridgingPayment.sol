// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {IBridgingPayment} from "src/interfaces/IBridgingPayment.sol";

/**
 * @dev BridgingPayment smart contract is responsible for collecting bridging fees
 *      for transferring tokens from Ethereum to Vara. This smart contract enables
 *      setting fee amount, updating owner address that will collect the fee, and
 *      setting `ERC20Manager` smart contract that will handle bridging requests.
 */
contract BridgingPayment is IBridgingPayment, Ownable {
    ERC20Manager private immutable ERC20_MANAGER;

    uint256 public fee;

    /**
     * @dev Initializes the BridgingPayment contract with the ERC20Manager address, fee amount, and initial owner.
     * @param _erc20Manager The ERC20Manager contract that will handle bridging requests.
     * @param _fee The initial fee amount required for bridging.
     * @param initialOwner The address that will receive the bridging fees.
     */
    constructor(ERC20Manager _erc20Manager, uint256 _fee, address initialOwner) Ownable(initialOwner) {
        ERC20_MANAGER = _erc20Manager;
        fee = _fee;
    }

    /**
     * @dev Returns the address of the ERC20Manager.
     */
    function erc20Manager() external view returns (address) {
        return address(ERC20_MANAGER);
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
    function payFee() external payable {
        if (msg.sender != address(ERC20_MANAGER)) {
            revert OnlyErc20Manager();
        }

        if (msg.value != fee) {
            revert IncorrectFeeAmount();
        }

        // forge-lint: disable-next-item(low-level-calls)
        (bool success,) = owner().call{value: msg.value}("");
        if (!success) {
            revert PayFeeFailed();
        }

        // forge-lint: disable-next-item(reentrancy-events)
        emit FeePaid();
    }
}
