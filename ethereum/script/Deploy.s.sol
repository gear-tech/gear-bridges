pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";

import {Test, console} from "forge-std/Test.sol";
import {Verifier} from "../src/Verifier.sol";
import {Verifier as VerifierMock} from "../src/mocks/VerifierMock.sol";
import {Relayer} from "../src/Relayer.sol";

import {ERC20Manager} from "../src/ERC20Manager.sol";
import {IERC20Manager} from "../src/interfaces/IERC20Manager.sol";

import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";

import {IVerifier} from "../src/interfaces/IVerifier.sol";

import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";

contract DeployScript is Script {
    Relayer public relayer;
    Verifier public verifier;
    ERC20Manager public erc20_manager;
    MessageQueue public message_queue;
    using Address for address;

    function setUp() public {}

    function run() public {
        vm.startBroadcast(vm.envUint("ETHEREUM_DEPLOYMENT_PRIVATE_KEY"));

        bytes32 vft_manager = vm.envBytes32("VFT_MANAGER");

        ProxyContract _relayer_proxy = new ProxyContract();
        ProxyContract _message_queue_proxy = new ProxyContract();
        ProxyContract _erc20_manager_proxy = new ProxyContract();

        IVerifier _verifier;

        try vm.envBool("MOCK") {
            if (vm.envBool("MOCK")) {
                console.log("Deploying MockVerifier");
                _verifier = IVerifier(address(new VerifierMock()));
            } else {
                console.log("Deploying Verifier");
                _verifier = IVerifier(address(new Verifier()));
            }
        } catch {
            console.log("Deploying Verifier");
            _verifier = IVerifier(address(new Verifier()));
        }

        Relayer _relayer = new Relayer(address(_verifier));
        ERC20Manager _erc20_manager = new ERC20Manager(
            address(_message_queue_proxy),
            vft_manager
        );

        MessageQueue _message_queue = new MessageQueue(address(_relayer_proxy));

        _relayer_proxy.upgradeToAndCall(address(_relayer), "");
        _erc20_manager_proxy.upgradeToAndCall(address(_erc20_manager), "");
        _message_queue_proxy.upgradeToAndCall(address(_message_queue), "");

        relayer = Relayer(address(_relayer_proxy));
        erc20_manager = ERC20Manager(address(_erc20_manager_proxy));
        message_queue = MessageQueue(address(_message_queue_proxy));
        verifier = Verifier(address(_verifier));

        console.log("Verifier:", address(verifier));
        console.log("Relayer:", address(_relayer));
        console.log("ERC20Manager:", address(_erc20_manager));
        console.log("MessageQueue:", address(_message_queue));
        console.log("Relayer Proxy:", address(relayer));
        console.log("ERC20Manager Proxy:", address(erc20_manager));
        console.log("MessageQueue Proxy:", address(message_queue));

        vm.stopBroadcast();
    }
}
