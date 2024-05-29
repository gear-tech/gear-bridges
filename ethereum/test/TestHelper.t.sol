pragma solidity ^0.8.20;

import {Test, console} from "forge-std/Test.sol";
import {ProxyContract} from "../src/ProxyContract.sol";

import {MessageQueue} from "../src/MessageQueue.sol";
import {ERC20Treasury} from "../src/ERC20Treasury.sol";
import {Verifier} from "../src/Verifier.sol";
import {Relayer} from "../src/Relayer.sol";
import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";

import {IVerifier} from "../src/interfaces/IVerifier.sol";


address constant OWNER = address(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
address constant USER = address(0x34A1D3fff3958843C43aD80F30b94c510645C316);

address constant ETH_ADDRESS_3 = address(0x0303030303030303030303030303030303030303);

address constant ETH_ADDRESS_5 = address(0x0505050505050505050505050505050505050505);


bytes32 constant VARA_ADDRESS_7 =
        bytes32(
            0x0707070707070707070707070707070707070707070707070707070707070707
        );

bytes32 constant VARA_ADDRESS_3 =
    bytes32(
        0x0303030303030303030303030303030303030303030303030303030303030303
    );

contract TestHelper is Test {
Relayer public relayer;
    IVerifier public verifier;
    ERC20Treasury public treasury;
    MessageQueue public message_queue;
    ERC20Mock public erc20_token;


    function setUp() public virtual {
        vm.startPrank(OWNER, OWNER);
        erc20_token = new ERC20Mock("wVARA");

        Verifier _verifier = new Verifier();

        Relayer _relayer = new Relayer();
        ERC20Treasury _treasury = new ERC20Treasury();
        MessageQueue _message_queue = new MessageQueue();

        ProxyContract _relayer_proxy = new ProxyContract(
            address(_relayer),
            bytes("")
        );

        ProxyContract _message_queue_proxy = new ProxyContract(
            address(_message_queue),
            bytes("")
        );
        ProxyContract _treasury_proxy = new ProxyContract(
            address(_treasury),
            bytes("")
        );

        relayer = Relayer(address(_relayer_proxy));
        treasury = ERC20Treasury(address(_treasury_proxy));
        message_queue = MessageQueue(address(_message_queue_proxy));

        verifier = IVerifier(address(_verifier));
        vm.stopPrank();

    }

}