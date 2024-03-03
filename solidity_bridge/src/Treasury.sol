pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Context} from "@openzeppelin/contracts/utils/Context.sol";

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {ITreasury} from "./interfaces/ITreasury.sol";
import {Constants} from "./libraries/Constants.sol";



contract Treasury is ITreasury, Context, AccessControl  {
    using SafeERC20 for IERC20;


    function initialize(address messageQueue) public {
        if(getRoleAdmin(Constants.ADMIN_ROLE) != DEFAULT_ADMIN_ROLE) revert AlreadyInitialized();
        _setRoleAdmin(Constants.ADMIN_ROLE, Constants.ADMIN_ROLE);
        _setRoleAdmin(Constants.MESSAGE_QUEUE_ROLE, Constants.ADMIN_ROLE);
        _grantRole(Constants.ADMIN_ROLE, msg.sender );
        _grantRole(Constants.MESSAGE_QUEUE_ROLE, messageQueue );
    }


    function deposit(address token, uint256 amount) public {
        IERC20(token).safeTransferFrom(_msgSender(), address(this), amount);
        emit Deposit(_msgSender(), token, amount);
    } 

    function withdraw(address token, address to, uint256 amount) internal {
        IERC20(token).safeTransfer(to, amount);
        emit Withdraw(token, to, amount);
    }

    fallback(bytes calldata data) onlyRole(Constants.MESSAGE_QUEUE_ROLE) external returns (bytes memory){
        (address token, address to, uint256 amount ) = abi.decode(data, (address, address, uint256));
        withdraw(token, to, amount);
        return( bytes("") );
    }

}