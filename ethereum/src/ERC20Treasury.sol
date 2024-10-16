pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Context} from "@openzeppelin/contracts/utils/Context.sol";

import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {IERC20Treasury} from "./interfaces/IERC20Treasury.sol";
import {VFT_GATEWAY_ADDRESS} from "./libraries/Environment.sol";
import {IMessageQueue, IMessageQueueReceiver, VaraMessage} from "./interfaces/IMessageQueue.sol";

import {BridgingPayment} from "./BridgingPayment.sol";

contract ERC20Treasury is IERC20Treasury, Context, IMessageQueueReceiver {
    using SafeERC20 for IERC20;

    address immutable MESSAGE_QUEUE_ADDRESS;

    constructor(address message_queue) {
        MESSAGE_QUEUE_ADDRESS = message_queue;
    }

    /** @dev Deposit token to `Treasury` using `safeTransferFrom`. Allowance needs to allow treasury
     * contract transferring `amount` of tokens. Emits `Deposit` event.
     *
     * @param token token address to deposit
     * @param amount quantity of deposited token
     * @param to destination of transfer on VARA network
     */
    function deposit(address token, uint256 amount, bytes32 to) public {
        IERC20(token).safeTransferFrom(tx.origin, address(this), amount);
        emit Deposit(tx.origin, to, token, amount);
    }

    /** @dev Request withdraw of tokens. This request must be sent by `MessageQueue` only. Expected
     * `payload` in `VaraMessage` consisits of these:
     *  - `receiver` - account to withdraw tokens to
     *  - `token` - token to withdraw
     *  - `amount` - amount of tokens to withdraw
     *
     * @param vara_msg `VaraMessage` received from MessageQueue.
     */
    function processVaraMessage(
        VaraMessage calldata vara_msg
    ) external returns (bool) {
        uint160 receiver;
        uint160 token;
        uint256 amount;
        if (msg.sender != MESSAGE_QUEUE_ADDRESS) {
            revert NotAuthorized();
        }

        if (vara_msg.data.length != 20 + 20 + 32) {
            revert BadArguments();
        }
        if (vara_msg.receiver != address(this)) {
            revert BadEthAddress();
        }
        if (vara_msg.sender != VFT_GATEWAY_ADDRESS) {
            revert BadVaraAddress();
        }

        assembly {
            receiver := shr(96, calldataload(0xC4))
            token := shr(96, calldataload(0xD8))
            amount := calldataload(0xEC)
        }
        IERC20(address(token)).safeTransfer(address(receiver), amount);
        emit Withdraw(address(receiver), address(token), amount);
        return true;
    }
}

contract ERC20TreasuryBridgingPayment is BridgingPayment {
    constructor(
        address _underlying,
        address _admin,
        uint256 _fee
    ) BridgingPayment(_underlying, _admin, _fee) {}

    function deposit(address token, uint256 amount, bytes32 to) public payable {
        deductFee();

        ERC20Treasury(underlying).deposit(token, amount, to);
    }
}
