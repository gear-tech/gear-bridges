pragma solidity ^0.8.13;


import {Address} from "@openzeppelin/contracts/utils/Address.sol";


import {Test, console} from "forge-std/Test.sol";
import {Prover} from "../src/mocks/ProverMock.sol";
import {Relayer} from "../src/Relayer.sol";

import {Treasury} from "../src/Treasury.sol";
import {ITreasury} from "../src/interfaces/ITreasury.sol";

import {MessageQueue} from "../src/MessageQueue.sol";
import {IMessageQueue, ContentMessage, VaraMessage, Hasher } from "../src/interfaces/IMessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {Constants} from "../src/libraries/Constants.sol";

import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";


contract MessageQueueTest is Test {
    Relayer public relayer;
    Prover public prover;
    Treasury public treasury;
    MessageQueue public message_queue;
    using Address for address;
    using Hasher for ContentMessage;
    
    ERC20Mock public erc20_token;

    uint256 private constant BLOCK_ID = 100;
    bytes32 private constant BLOCK_MERKLE_ROOT = keccak256(bytes("Block100"));
    bytes32 private constant VARA_ID = keccak256(bytes("VARA_ID"));

    function setUp() public {
        Prover _prover = new Prover();
        Relayer _relayer = new Relayer();
        Treasury _treasury = new Treasury();
        MessageQueue _message_queue = new MessageQueue();
        
        ProxyContract _relayer_proxy = new ProxyContract( address(_relayer), abi.encodeWithSignature("initialize(address)", address(_prover) )); 
        
        ProxyContract _message_queue_proxy = new ProxyContract( address(_message_queue), abi.encodeWithSignature("initialize(address,address)", address(_prover), address(_relayer_proxy) )); 
        ProxyContract _treasury_proxy = new ProxyContract(address(_treasury), abi.encodeWithSignature("initialize(address)", address(_message_queue_proxy)  ));

        relayer = Relayer(address(_relayer_proxy));
        treasury = Treasury(address(_treasury_proxy));
        message_queue = MessageQueue(address(_message_queue_proxy) );
        prover = Prover(address(_prover));

        erc20_token = new ERC20Mock("wVARA");


        uint256 amount = 100 * (10 ** 18);
        erc20_token.approve(address(treasury), amount);
        treasury.deposit(address(erc20_token), amount);


        relayer.add_merkle_root_with_block(BLOCK_ID, BLOCK_MERKLE_ROOT, bytes(""));


    }


    function testWithdraw() public {
        ContentMessage memory content_message = ContentMessage({ 
            eth_address : address(treasury), 
            vara_address : VARA_ID, 
            nonce : 1, 
            data : abi.encode(  address(erc20_token), address(this), 100 ) 
        });

        VaraMessage memory vara_message = VaraMessage({
            block_number : BLOCK_ID,
            content : content_message,
            proof : bytes("")
        });

        bytes32 messageHash = content_message.hash();

        message_queue.process_message(vara_message);

        vm.expectRevert(  abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, messageHash) );
        message_queue.process_message(vara_message);


    }


}