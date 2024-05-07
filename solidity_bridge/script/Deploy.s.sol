pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";


import {Test, console} from "forge-std/Test.sol";
import {Verifier} from "../src/Verifier.sol";
import {Relayer} from "../src/Relayer.sol";

import {Treasury} from "../src/Treasury.sol";
import {ITreasury} from "../src/interfaces/ITreasury.sol";

import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {Constants} from "../src/libraries/Constants.sol";

import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";


contract DeployScript is Script {
    Relayer public relayer;
    Verifier public verifier;
    Treasury public treasury;
    MessageQueue public message_queue;
    using Address for address;


    function setUp() public {}

    function run() public {
        vm.broadcast();
        Verifier _verifier = new Verifier();
        vm.broadcast();
        Relayer _relayer = new Relayer();
        vm.broadcast();
        Treasury _treasury = new Treasury();

        vm.broadcast();
        MessageQueue _message_queue = new MessageQueue();

        vm.broadcast();
        ProxyContract _relayer_proxy = new ProxyContract(address(_relayer), abi.encodeWithSignature("initialize(address)", address(_verifier)));

        vm.broadcast();
        ProxyContract _message_queue_proxy = new ProxyContract(address(_message_queue), abi.encodeWithSignature("initialize(address)", address(_relayer_proxy)));

        vm.broadcast();
        ProxyContract _treasury_proxy = new ProxyContract(address(_treasury), abi.encodeWithSignature("initialize(address)", address(_message_queue_proxy)));

        relayer = Relayer(address(_relayer_proxy));
        treasury = Treasury(address(_treasury_proxy));
        message_queue = MessageQueue(address(_message_queue_proxy));
        verifier = Verifier(address(_verifier));

        console.log("Prover:", address(verifier));
        console.log("Relayer:", address(_relayer));
        console.log("Treasury:", address(_treasury));
        console.log("MessageQueue:", address(_treasury));
        console.log("Relayer Proxy:", address(relayer));
        console.log("Treasury Proxy:", address(treasury));
        console.log("MessageQueue Proxy:", address(message_queue));

    }
}

