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


    function initialize(address messageQueue) public {
        if (getRoleAdmin(Constants.MESSAGE_QUEUE_ROLE) != DEFAULT_ADMIN_ROLE) revert AlreadyInitialized();
        _setRoleAdmin(Constants.MESSAGE_QUEUE_ROLE, Constants.ADMIN_ROLE);
        _grantRole(Constants.MESSAGE_QUEUE_ROLE, messageQueue);
    }


    function deposit(address token, uint256 amount) public {
        IERC20(token).safeTransferFrom(_msgSender(), address(this), amount);
        emit Deposit(_msgSender(), token, amount);
    }

    function processVaraMessage(VaraMessage calldata vara_msg) onlyRole(Constants.MESSAGE_QUEUE_ROLE) external returns (bool){
        uint160 receiver;
        uint160 token;
        uint256 amount;

        if (vara_msg.data.length != 20 + 20 + 16) {
            revert BadArguments();
        }
        if (vara_msg.eth_address != address(this)) {
            revert BadEthAddress();
        }
        if (vara_msg.vara_address != VARA_TRUSTED_ADDRESS) {
            revert BadVaraAddress();
        }

        assembly {
            receiver := shr(96, calldataload(0xC4))
            token := shr(96, calldataload(0xD8))
            amount := shr(128, calldataload(0xEC))
        }
        _withdraw(address(token), address(receiver), amount);
        return true;
    }

    function _withdraw(address token, address to, uint256 amount) internal {
        IERC20(token).safeTransfer(to, amount);
        emit Withdraw(token, to, amount);
    }


}