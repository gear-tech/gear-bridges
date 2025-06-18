// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test, console} from "forge-std/Test.sol";
import {IRelayer} from "../src/interfaces/IRelayer.sol";
import {IVerifier} from "../src/interfaces/IVerifier.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {WrappedVara} from "../src/erc20/WrappedVara.sol";
import {MessageQueue} from "../src/MessageQueue.sol";
import {ERC20Manager} from "../src/ERC20Manager.sol";
import {VerifierMock} from "../src/mocks/VerifierMock.sol";
import {Relayer} from "../src/Relayer.sol";

address constant OWNER = address(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
address constant USER = address(0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496);

bytes32 constant VFT_MANAGER_ADDRESS = bytes32(
    0x0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A
);

address constant ETH_ADDRESS_3 = address(
    0x0303030303030303030303030303030303030303
);

address constant ETH_ADDRESS_5 = address(
    0x0505050505050505050505050505050505050505
);

bytes32 constant VARA_ADDRESS_7 = bytes32(
    0x0707070707070707070707070707070707070707070707070707070707070707
);

bytes32 constant VARA_ADDRESS_3 = bytes32(
    0x0303030303030303030303030303030303030303030303030303030303030303
);

contract TestHelper is Test {
    Relayer public relayer;
    IVerifier public verifier;
    ERC20Manager public erc20_manager;
    MessageQueue public message_queue;
    WrappedVara public erc20_token;

    function setUp() public virtual {
        vm.startPrank(OWNER, OWNER);
        ProxyContract _relayer_proxy = new ProxyContract();
        ProxyContract _message_queue_proxy = new ProxyContract();
        ProxyContract _treasury_proxy = new ProxyContract();

        erc20_token = new WrappedVara(OWNER);
        erc20_token.mint(OWNER, type(uint256).max);

        VerifierMock _verifier = new VerifierMock();

        Relayer _relayer = new Relayer(_verifier);
        ERC20Manager _erc20_manager = new ERC20Manager(
            address(_message_queue_proxy),
            bytes32(0)
        );
        MessageQueue _message_queue = new MessageQueue(IRelayer(address(_relayer_proxy)));

        _relayer_proxy.upgradeToAndCall(address(_relayer), "");
        _treasury_proxy.upgradeToAndCall(address(_erc20_manager), "");
        _message_queue_proxy.upgradeToAndCall(address(_message_queue), "");

        relayer = Relayer(address(_relayer_proxy));
        erc20_manager = ERC20Manager(address(_treasury_proxy));
        message_queue = MessageQueue(address(_message_queue_proxy));

        verifier = IVerifier(address(_verifier));
        vm.stopPrank();
    }
}
