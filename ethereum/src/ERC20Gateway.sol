pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {IERC20Gateway} from "./interfaces/IERC20Gateway.sol";
import {VFT_TREASURY_ADDRESS} from "./libraries/Environment.sol";
import {IMessageQueue, IMessageQueueReceiver, VaraMessage} from "./interfaces/IMessageQueue.sol";
import {ERC20VaraSupply} from "./ERC20VaraSupply.sol";

import {BridgingPayment} from "./BridgingPayment.sol";

contract ERC20Gateway is IERC20Gateway, IMessageQueueReceiver {
    address immutable MESSAGE_QUEUE_ADDRESS;

    constructor(address message_queue) {
        MESSAGE_QUEUE_ADDRESS = message_queue;
    }

    /** @dev Request token bridging. When the bridging is requested tokens are burned
     * from account that've sent transaction and `BridgingRequested` event is emitted that later can be verified
     * on other side of bridge. Allowance needs to allow gateway contract transferring `amount` of tokens.
     *
     * @param token token address to transfer over bridge
     * @param amount quantity of tokens to transfer over bridge
     * @param to destination of transfer on VARA network
     */
    function requestBridging(address token, uint256 amount, bytes32 to) public {
        ERC20VaraSupply(token).burnFrom(tx.origin, amount);
        emit BridgingRequested(tx.origin, to, token, amount);
    }

    /** @dev Accept bridging request made on other side of bridge.
     * This request must be sent by `MessageQueue` only. When such a request is accepted, tokens
     * are minted to the corresponding account address, specified in `payload`.
     *
     * Expected `payload` consisits of these:
     *  - `receiver` - account to mint tokens to
     *  - `token` - token to mint
     *  - `amount` - amount of tokens to mint
     *
     * Expected sender should be `vft-treasury` program on gear.
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
        if (sender != VFT_TREASURY_ADDRESS) {
            revert BadVaraAddress();
        }

        address receiver = abi.decode(payload[:20], (address));
        address token = abi.decode(payload[20:40], (address));
        uint256 amount = abi.decode(payload[40:], (uint256));

        ERC20VaraSupply(token).mint(receiver, amount);
        emit BridgingAccepted(receiver, token, amount);

        return true;
    }
}

contract ERC20GatewayBridgingPayment is BridgingPayment {
    constructor(
        address _underlying,
        address _admin,
        uint256 _fee
    ) BridgingPayment(_underlying, _admin, _fee) {}

    /** @dev Call `requestBridging` function from `ERC20Gateway` contract. This function also
     * deducting some fee in native tokens from such a call. For further info see `ERC20Gateway::requestBridging`.
     */
    function requestBridging(
        address token,
        uint256 amount,
        bytes32 to
    ) public payable {
        deductFee();

        ERC20Gateway(underlying).requestBridging(token, amount, to);
    }
}
