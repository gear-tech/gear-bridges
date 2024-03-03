pragma solidity ^0.8.24;



interface ITreasury {
    error AlreadyInitialized();

    event Deposit(address indexed token, address indexed from, uint256 amount);
    event Withdraw(address indexed token, address indexed to, uint256 amount);
}