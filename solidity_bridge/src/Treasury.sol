pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Context} from "@openzeppelin/contracts/utils/Context.sol";

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {ITreasury, WithdrawMessage} from "./interfaces/ITreasury.sol";
import {Constants} from "./libraries/Constants.sol";
import {IMessageQueue, IMessageQueueReceiver, VaraMessage} from "./interfaces/IMessageQueue.sol";


contract Treasury is ITreasury, Context, AccessControl, IMessageQueueReceiver {
    using SafeERC20 for IERC20;

    bytes32 private constant VARA_TRUSTED_ADDRESS = bytes32(0x0707070707070707070707070707070707070707070707070707070707070707);


    /** @dev initializes contract. Should be called through proxy immediately after deployment
    *
    * @param message_queue - address of message queue that is authorized to send messages
    */
    function initialize(address message_queue) public {
        if (getRoleAdmin(Constants.MESSAGE_QUEUE_ROLE) != DEFAULT_ADMIN_ROLE) revert AlreadyInitialized();
        _setRoleAdmin(Constants.MESSAGE_QUEUE_ROLE, Constants.ADMIN_ROLE);
        _grantRole(Constants.MESSAGE_QUEUE_ROLE, message_queue);
    }


    /** @dev deposit token to Treasury using transferFrom. Allowance needs to allow treasury contract transferring
    * amount of tokens. Emits Deposit event.
    *
    * @param token token address to deposit
    * @param amount quantity of deposited token
    */
    function deposit(address token, uint256 amount) public {
        IERC20(token).safeTransferFrom(_msgSender(), address(this), amount);
        emit Deposit(_msgSender(), token, amount);
    }

    /** @dev callback function receiving VaraMessage as a parameter. Validates sender and receiver and transfers Tokens
    * to specified address. Emits Withdraw event.
    *
    * @param vara_msg VaraMessage received from MessageQueue. Please check original documentation for description.
    */
    function processVaraMessage(VaraMessage calldata vara_msg) onlyRole(Constants.MESSAGE_QUEUE_ROLE) external returns (bool){
        uint160 receiver;
        uint160 token;
        uint256 amount;

        if (vara_msg.data.length != 20 + 20 + 16) {
            revert BadArguments();
        }
        if (vara_msg.receiver != address(this)) {
            revert BadEthAddress();
        }
        if (vara_msg.sender != VARA_TRUSTED_ADDRESS) {
            revert BadVaraAddress();
        }

        assembly {
            receiver := shr(96, calldataload(0xC4))
            token := shr(96, calldataload(0xD8))
            amount := shr(128, calldataload(0xEC))
        }
        IERC20(address(token)).safeTransfer(address(receiver), amount);
        emit Withdraw(address(token), address(receiver), amount);
        return true;
    }


}