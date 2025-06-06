// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

interface IBridgingPayment {
    event FeePaid();

    function payFee() external payable;

    function setFee(uint256 newFee) external;

    function erc20Manager() external view returns (address);

    function fee() external view returns (uint256);
}
