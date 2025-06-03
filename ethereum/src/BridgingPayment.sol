// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IBridgingPayment} from "./interfaces/IBridgingPayment.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

contract BridgingPayment is IBridgingPayment, Ownable {
    error OnlyErc20Manager();
    error IncorrectFeeAmount();

    address public immutable erc20Manager;
    uint256 public fee;

    constructor(address _erc20Manager, uint256 _fee, address initialOwner) Ownable(initialOwner) {
        erc20Manager = _erc20Manager;
        fee = _fee;
    }

    function payFee() external payable {
        if (msg.sender != erc20Manager) {
            revert OnlyErc20Manager();
        }

        if (msg.value != fee) {
            revert IncorrectFeeAmount();
        }

        payable(owner()).transfer(msg.value);

        emit FeePaid();
    }

    function setFee(uint256 _fee) external onlyOwner {
        fee = _fee;
    }
}
