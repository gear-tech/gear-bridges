pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";

import {IERC20Manager} from "./interfaces/IERC20Manager.sol";
import {IMessageQueueReceiver} from "./interfaces/IMessageQueue.sol";
import {ERC20VaraSupply} from "./ERC20VaraSupply.sol";
import {BridgingPayment} from "./BridgingPayment.sol";

contract ERC20Manager is IERC20Manager, IMessageQueueReceiver {
    using SafeERC20 for IERC20;

    address immutable MESSAGE_QUEUE_ADDRESS;
    bytes32 immutable VFT_MANAGER_ADDRESS;

    constructor(address message_queue, bytes32 vft_manager) {
        MESSAGE_QUEUE_ADDRESS = message_queue;
        VFT_MANAGER_ADDRESS = vft_manager;
    }

    /** @dev Request token bridging. When the bridging is requested tokens are burned/locked (based on `supply_type`)
     * from account that've sent transaction and `BridgingRequested` event is emitted that later can be verified
     * on other side of bridge.
     *
     * `supply_type` can be either 0 ot 1.
     * - if 0: supply is on ethereum, so mint/burn on gear side and lock/unlock on ethereum side
     * - if 1: supply is on gear, so lock/unlock on gear side and mint/burn on ethereum side
     *
     * @param token token address to transfer over bridge
     * @param amount quantity of tokens to transfer over bridge
     * @param to destination of transfer on gear
     * @param supply_type type of the token supply
     */
    function requestBridging(
        address token,
        uint256 amount,
        bytes32 to,
        uint8 supply_type
    ) public {
        // TODO: Actually `supply_type` can be determined automatically.
        // It can be done based on the fact that tokens with supply on gear side can appear on ethereum only after bridging,
        // so they shoul've been processed in `processVaraMessage` before. For tokens with supply on ethereum side
        // they cannot be bridged from gear to ethereum without first appearing in `requestBridging`.

        if (supply_type == 0) {
            IERC20(token).safeTransferFrom(tx.origin, address(this), amount);
        } else if (supply_type == 1) {
            ERC20VaraSupply(token).burnFrom(tx.origin, amount);
        } else {
            revert UnsupportedTokenSupply();
        }

        emit BridgingRequested(tx.origin, to, token, amount);
    }

    /** @dev Accept bridging request made on other side of bridge.
     * This request must be sent by `MessageQueue` only. When such a request is accepted, tokens
     * are minted to the corresponding account address, specified in `payload`.
     *
     * Expected `payload` consisits of these:
     *  - `supply_type` - type of the supply
     *  - `receiver` - account to mint tokens to
     *  - `token` - token to mint
     *  - `amount` - amount of tokens to mint
     *
     * `supply_type` can be either 0 ot 1.
     * - if 0: supply is on ethereum, so mint/burn on gear side and lock/unlock on ethereum side
     * - if 1: supply is on gear, so lock/unlock on gear side and mint/burn on ethereum side
     *
     * Expected sender should be `vft-manager` program on gear.
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
        if (payload.length != 1 + 20 + 20 + 32) {
            revert BadArguments();
        }
        // TODO: Set VFT_MANAGER_ADDRESS in constructor.
        if (sender != VFT_MANAGER_ADDRESS) {
            revert BadVftManagerAddress();
        }

        uint8 supply_type = uint8(bytes1(payload[:1]));
        address receiver = address(bytes20(payload[1:21]));
        address token = address(bytes20(payload[21:41]));
        uint256 amount = uint256(bytes32(payload[41:]));

        if (supply_type == 0) {
            IERC20(token).safeTransfer(receiver, amount);
        } else if (supply_type == 1) {
            ERC20VaraSupply(token).mint(receiver, amount);
        } else {
            revert UnsupportedTokenSupply();
        }

        emit BridgingAccepted(receiver, token, amount);

        return true;
    }
}

contract ERC20ManagerBridgingPayment is BridgingPayment {
    constructor(
        address _underlying,
        address _admin,
        uint256 _fee
    ) BridgingPayment(_underlying, _admin, _fee) {}

    /** @dev Call `requestBridging` function from `ERC20Manager` contract. This function also
     * deducting some fee in native tokens from such a call. For further info see `ERC20Manager::requestBridging`.
     */
    function requestBridging(
        address token,
        uint256 amount,
        bytes32 to,
        uint8 supply_type
    ) public payable {
        deductFee();

        ERC20Manager(underlying).requestBridging(
            token,
            amount,
            to,
            supply_type
        );
    }
}
