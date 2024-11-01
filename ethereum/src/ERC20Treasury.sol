pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {IERC20Treasury} from "./interfaces/IERC20Treasury.sol";
import {VFT_GATEWAY_ADDRESS} from "./libraries/Environment.sol";
import {IMessageQueue, IMessageQueueReceiver, VaraMessage} from "./interfaces/IMessageQueue.sol";

import {BridgingPayment} from "./BridgingPayment.sol";

contract ERC20Treasury is IERC20Treasury, IMessageQueueReceiver {
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

    /** @dev Request withdraw of tokens. This request must be sent by `MessageQueue` only.
     *
     * Expected `payload` consisits of these:
     *  - `receiver` - account to withdraw tokens to
     *  - `token` - token to withdraw
     *  - `amount` - amount of tokens to withdraw
     *
     * Expected sender should be `vft-gateway` program on gear.
     *
     * @param sender sender of message on the gear side.
     * @param payload payload of the message.
     */
    function processVaraMessage(
        bytes32 sender,
        bytes calldata payload
    ) external returns (bool) {
        if (msg.sender != MESSAGE_QUEUE_ADDRESS) {
            revert NotAuthorized();
        }
        if (payload.length != 20 + 20 + 32) {
            revert BadArguments();
        }
        if (sender != VFT_GATEWAY_ADDRESS) {
            revert BadVaraAddress();
        }

        address receiver = address(bytes20(payload[:20]));
        address token = address(bytes20(payload[20:40]));
        uint256 amount = uint256(bytes32(payload[40:]));

        IERC20(token).safeTransfer(receiver, amount);
        emit Withdraw(receiver, token, amount);

        return true;
    }
}

contract ERC20TreasuryBridgingPayment is BridgingPayment {
    constructor(
        address _underlying,
        address _admin,
        uint256 _fee
    ) BridgingPayment(_underlying, _admin, _fee) {}

    /** @dev Call `deposit` function from `ERC20Treasury` contract. This function also
     * deducting some fee in native tokens from such a call. For further info see `ERC20Treasury::deposit`.
     */
    function deposit(address token, uint256 amount, bytes32 to) public payable {
        deductFee();

        ERC20Treasury(underlying).deposit(token, amount, to);
    }
}
