pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Context} from "@openzeppelin/contracts/utils/Context.sol";

import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {IERC20Gateway} from "./interfaces/IERC20Gateway.sol";
import {VFT_TREASURY_ADDRESS} from "./libraries/Environment.sol";
import {IMessageQueue, IMessageQueueReceiver, VaraMessage} from "./interfaces/IMessageQueue.sol";
import {ERC20VaraSupply} from "./ERC20VaraSupply.sol";

import {BridgingPayment} from "./BridgingPayment.sol";

contract ERC20Gateway is IERC20Gateway, Context, IMessageQueueReceiver {
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
        ERC20VaraSupply(token).burnFrom(_msgSender(), amount);
        emit BridgingRequested(_msgSender(), to, token, amount);
    }

    /** @dev Accept bridging request made on other side of bridge.
     * This request can be sent by `MessageQueue` only. When such a request is accpeted, tokens
     * are minted to the corresponding account address, specified in `vara_msg`.
     *
     * Expected `payload` in `VaraMessage` consisits of these:
     *  - `receiver` - account to mint tokens to
     *  - `token` - token to mint
     *  - `amount` - amount of tokens to mint
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
        if (vara_msg.sender != VFT_TREASURY_ADDRESS) {
            revert BadVaraAddress();
        }

        assembly {
            receiver := shr(96, calldataload(0xC4))
            token := shr(96, calldataload(0xD8))
            amount := calldataload(0xEC)
        }

        ERC20VaraSupply(address(token)).mint(address(receiver), amount);

        emit BridgingAccepted(address(receiver), address(token), amount);
        return true;
    }
}

contract ERC20GatewayBridgingPayment is BridgingPayment {
    constructor(
        address _underlying,
        address _admin,
        uint256 _fee
    ) BridgingPayment(_underlying, _admin, _fee) {}

    function requestBridging(
        address token,
        uint256 amount,
        bytes32 to
    ) public payable {
        deductFee();

        ERC20Gateway(underlying).requestBridging(token, amount, to);
    }
}
