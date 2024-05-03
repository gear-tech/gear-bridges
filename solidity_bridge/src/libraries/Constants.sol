pragma solidity ^0.8.13;

library Constants {
    bytes32 public constant ADMIN_ROLE = keccak256("ADMIN_ROLE");
    bytes32 public constant MESSAGE_QUEUE_ROLE = keccak256("MESSAGE_QUEUE_ROLE");
}