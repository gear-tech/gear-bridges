pragma solidity ^0.8.13;


import {Address} from "@openzeppelin/contracts/utils/Address.sol";


import {Test, console} from "forge-std/Test.sol";
import {Prover} from "../src/Prover.sol";
import {Relayer} from "../src/Relayer.sol";

import {Treasury} from "../src/Treasury.sol";
import {ITreasury} from "../src/interfaces/ITreasury.sol";

import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {Constants} from "../src/libraries/Constants.sol";

import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";



contract TreasuryTest is Test {
    Relayer public relayer;
    Prover public prover;
    Treasury public treasury;
    MessageQueue public message_queue;
    using Address for address;
    
    ERC20Mock public erc20_token;

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
    }

    function test_messageQueueRole() public {
        address not_admin = address(0x5124fcC2B3F99F571AD67D075643C743F38f1C34);

        // from pranker
        vm.prank(not_admin, not_admin );
        vm.expectRevert();
        treasury.grantRole(Constants.ADMIN_ROLE, not_admin);

        bytes32 role_admin = treasury.getRoleAdmin(Constants.MESSAGE_QUEUE_ROLE);
        console.logBytes32(role_admin);

        treasury.grantRole(Constants.MESSAGE_QUEUE_ROLE, not_admin);

        treasury.grantRole(Constants.ADMIN_ROLE, not_admin);
    }


    function test_deposit() public {
        uint256 amount = 100 * (10 ** 18);
        erc20_token.approve(address(treasury), amount );
        treasury.deposit(address(erc20_token), amount);
    }

    function test_withdraw() public {
        uint256 amount = 100 * (10 ** 18);
        erc20_token.approve(address(treasury), amount );
        treasury.deposit(address(erc20_token), amount);

        vm.expectRevert( );
        address(treasury).functionCall( abi.encode(address(erc20_token), address(this), amount) );

        vm.prank( address(message_queue) );
        address(treasury).functionCall( abi.encode(address(erc20_token), address(this), amount) );

    }



}