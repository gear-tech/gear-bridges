// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";

import {IBridgingPayment} from "./interfaces/IBridgingPayment.sol";
import {IERC20Manager} from "./interfaces/IERC20Manager.sol";
import {IMessageQueueReceiver} from "./interfaces/IMessageQueue.sol";
import {ERC20GearSupply} from "../src/erc20/ERC20GearSupply.sol";
import {BridgingPayment} from "./BridgingPayment.sol";

contract ERC20Manager is IERC20Manager, IMessageQueueReceiver {
    using SafeERC20 for IERC20;

    address immutable MESSAGE_QUEUE_ADDRESS;
    bytes32 immutable VFT_MANAGER_ADDRESS;

    mapping(address => SupplyType) tokenSupplyType;

    constructor(address message_queue, bytes32 vft_manager) {
        MESSAGE_QUEUE_ADDRESS = message_queue;
        VFT_MANAGER_ADDRESS = vft_manager;
    }

    /** @dev Request token bridging. When the bridging is requested tokens are burned/locked (based on the type of supply)
     * from account that've sent transaction and `BridgingRequested` event is emitted that later can be verified
     * on other side of bridge.
     *
     * @param token token address to transfer over bridge
     * @param amount quantity of tokens to transfer over bridge
     * @param to destination of transfer on gear
     */
    function requestBridging(address token, uint256 amount, bytes32 to) public {
        SupplyType supply_type = tokenSupplyType[token];

        if (supply_type == SupplyType.Gear) {
            ERC20GearSupply(token).burnFrom(msg.sender, amount);
        } else {
            if (supply_type == SupplyType.Unknown) {
                tokenSupplyType[token] = SupplyType.Ethereum;
            }

            IERC20(token).safeTransferFrom(msg.sender, address(this), amount);
        }

        emit BridgingRequested(msg.sender, to, token, amount);
    }

    function requestBridgingPayingFee(address token, uint256 amount, bytes32 to, address bridgingPayment) public payable {
        IBridgingPayment(bridgingPayment).payFee{value: msg.value}();
        requestBridging(token, amount, to);
    }

    /** @dev Accept bridging request made on other side of bridge.
     * This request must be sent by `MessageQueue` only. When such a request is accepted, tokens
     * are minted/unlocked to the corresponding account address, specified in `payload`.
     *
     * Expected `payload` consisits of these:
     *  - `receiver` - account to mint tokens to
     *  - `token` - token to mint
     *  - `amount` - amount of tokens to mint
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
        if (payload.length != 20 + 20 + 32) {
            revert BadArguments();
        }
        if (sender != VFT_MANAGER_ADDRESS) {
            revert BadVftManagerAddress();
        }

        address receiver = address(bytes20(payload[0:20]));
        address token = address(bytes20(payload[20:40]));
        uint256 amount = uint256(bytes32(payload[40:]));

        SupplyType supply_type = tokenSupplyType[token];

        if (supply_type == SupplyType.Ethereum) {
            IERC20(token).safeTransfer(receiver, amount);
        } else {
            if (supply_type == SupplyType.Unknown) {
                tokenSupplyType[token] = SupplyType.Gear;
            }

            ERC20GearSupply(token).mint(receiver, amount);
        }

        emit BridgingAccepted(receiver, token, amount);

        return true;
    }

    function getTokenSupplyType(
        address token
    ) public view returns (SupplyType) {
        return tokenSupplyType[token];
    }
}
