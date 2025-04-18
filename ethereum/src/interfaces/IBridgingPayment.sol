pragma solidity ^0.8.24;

interface IBridgingPayment {
    event FeePaid();

    function payFee() external payable;

    function setFee(uint256 newFee) external;

    function erc20Manager() external view returns (address);

    function fee() external view returns (uint256);
}
