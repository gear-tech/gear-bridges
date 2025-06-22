// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";

import {IBridgingPayment} from "./interfaces/IBridgingPayment.sol";
import {IERC20Manager} from "./interfaces/IERC20Manager.sol";
import {IMessageQueueProcessor} from "./interfaces/IMessageQueueProcessor.sol";
import {IERC20Burnable} from "./interfaces/IERC20Burnable.sol";
import {IERC20Mintable} from "./interfaces/IERC20Mintable.sol";
import {BridgingPayment} from "./BridgingPayment.sol";

contract ERC20Manager is IMessageQueueProcessor, IERC20Manager {
    using SafeERC20 for IERC20;

    address immutable MESSAGE_QUEUE;
    bytes32 immutable VFT_MANAGER;

    /**
     * @dev Size of the withdraw message.
     *
     *      ```solidity
     *      struct WithdrawMessage {
     *          address receiver; // 20 bytes
     *          address token; // 20 bytes
     *          uint256 amount; // 32 bytes
     *          bytes32 gearTokensSender; // 32 bytes
     *      }
     *      ```
     */
    uint256 private constant WITHDRAW_MESSAGE_SIZE = 104; //20 + 20 + 32 + 32

    mapping(address token => SupplyType supplyType) private tokenSupplyType;

    /**
     * @dev Initializes the ERC20Manager contract with the message queue and VFT manager addresses.
     * @param messageQueue The address of the message queue contract.
     * @param vftManager The address of the VFT manager contract (on Vara Network).
     */
    constructor(address messageQueue, bytes32 vftManager) {
        MESSAGE_QUEUE = messageQueue;
        VFT_MANAGER = vftManager;
    }

    /**
     * @dev Request token bridging. When the bridging is requested tokens are burned/locked (based on the type of supply)
     * from account that've sent transaction and `BridgingRequested` event is emitted that later can be verified
     * on other side of bridge.
     *
     * @param token token address to transfer over bridge
     * @param amount quantity of tokens to transfer over bridge
     * @param to destination of transfer on gear
     */
    function requestBridging(address token, uint256 amount, bytes32 to) public {
        SupplyType supplyType = tokenSupplyType[token];

        if (supplyType == SupplyType.Gear) {
            IERC20Burnable(token).burnFrom(msg.sender, amount);
        } else {
            if (supplyType == SupplyType.Unknown) {
                tokenSupplyType[token] = SupplyType.Ethereum;
            }

            IERC20(token).safeTransferFrom(msg.sender, address(this), amount);
        }

        emit BridgingRequested(msg.sender, to, token, amount);
    }

    function requestBridgingPayingFee(address token, uint256 amount, bytes32 to, address bridgingPayment)
        public
        payable
    {
        IBridgingPayment(bridgingPayment).payFee{value: msg.value}();
        requestBridging(token, amount, to);
    }

    function requestBridgingPayingFeeWithPermit(
        address token,
        uint256 amount,
        bytes32 to,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s,
        address bridgingPayment
    ) public payable {
        IBridgingPayment(bridgingPayment).payFee{value: msg.value}();
        try IERC20Permit(token).permit(msg.sender, address(this), amount, deadline, v, r, s) {} catch {}
        requestBridging(token, amount, to);
    }

    /**
     * @dev Accept bridging request made on other side of bridge.
     *
     *      This request must be sent by `MessageQueue` only. When such a request is accepted, tokens
     *      are minted/unlocked to the corresponding account address, specified in `payload`.
     *
     *      Expected `payload` consisits of these:
     *      - `receiver` - account to mint tokens to
     *      - `token` - token to mint
     *      - `amount` - amount of tokens to mint
     *      - `gearTokensSender` - sender of tokens on gear side
     *
     *      Expected sender should be `vft-manager` program on gear.
     *
     * @param source Source of message on the gear side.
     * @param payload Payload of the message.
     */
    function processMessage(bytes32 source, bytes calldata payload) external {
        if (msg.sender != MESSAGE_QUEUE) {
            revert NotAuthorized();
        }

        bytes32 governance = bytes32(0);
        if (source == VFT_MANAGER) {
            if (payload.length != WITHDRAW_MESSAGE_SIZE) {
                revert BadArguments();
            }

            address receiver = address(bytes20(payload[0:20]));
            address token = address(bytes20(payload[20:40]));
            uint256 amount = uint256(bytes32(payload[40:72]));
            bytes32 gearTokensSender = bytes32(payload[72:104]);

            SupplyType supplyType = tokenSupplyType[token];

            if (supplyType == SupplyType.Ethereum) {
                IERC20(token).safeTransfer(receiver, amount);
            } else {
                if (supplyType == SupplyType.Unknown) {
                    tokenSupplyType[token] = SupplyType.Gear;
                }

                IERC20Mintable(token).mint(receiver, amount);
            }

            emit BridgingAccepted(receiver, token, amount, gearTokensSender);
        } else if (source == governance) {
            //TODO: some special logic for governance
        } else {
            revert BadSender();
        }
    }

    function getTokenSupplyType(address token) external view returns (SupplyType) {
        return tokenSupplyType[token];
    }
}
