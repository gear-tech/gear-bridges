pragma solidity ^0.8.24;

import {IBridgingPayment} from "./interfaces/IBridgingPayment.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

contract BridgingPayment is IBridgingPayment, Ownable {
    address public immutable erc20Manager;
    uint256 public fee;

    constructor(address _erc20Manager, uint256 _fee, address initialOwner) Ownable(initialOwner) {
        erc20Manager = _erc20Manager;
        fee = _fee;
    }

    function payFee() external payable {
        require(msg.sender == erc20Manager, "only erc20 manager may call fee payment");

        require(msg.value == fee, "incorrect fee amount");

        payable(owner()).transfer(msg.value);

        emit FeePaid();
    }

    function setFee(uint256 _fee) external onlyOwner {
        fee = _fee;
    }
}
