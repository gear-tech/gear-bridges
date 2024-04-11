pragma solidity ^0.8.13;


import {Address} from "@openzeppelin/contracts/utils/Address.sol";


import {Test, console} from "forge-std/Test.sol";
import {Prover} from "../src/mocks/ProverMock.sol";
import {Relayer} from "../src/Relayer.sol";

import {Treasury} from "../src/Treasury.sol";
import {ITreasury} from "../src/interfaces/ITreasury.sol";

import {MessageQueue} from "../src/MessageQueue.sol";
import {IMessageQueue, ContentMessage, VaraMessage, Hasher} from "../src/interfaces/IMessageQueue.sol";
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

    uint256 private constant BLOCK_ID = 274;
    bytes32 private constant BLOCK_MERKLE_ROOT = keccak256(bytes("Block100"));
    bytes32 private constant VARA_ID = keccak256(bytes("VARA_ID"));

    function setUp() public {
        Prover _prover = new Prover();
        Relayer _relayer = new Relayer();
        Treasury _treasury = new Treasury();
        MessageQueue _message_queue = new MessageQueue();

        ProxyContract _relayer_proxy = new ProxyContract(address(_relayer), abi.encodeWithSignature("initialize(address)", address(_prover)));

        ProxyContract _message_queue_proxy = new ProxyContract(address(_message_queue), abi.encodeWithSignature("initialize(address,address)", address(_prover), address(_relayer_proxy)));
        ProxyContract _treasury_proxy = new ProxyContract(address(_treasury), abi.encodeWithSignature("initialize(address)", address(_message_queue_proxy)));

        relayer = Relayer(address(_relayer_proxy));
        treasury = Treasury(address(_treasury_proxy));
        message_queue = MessageQueue(address(_message_queue_proxy));
        prover = Prover(address(_prover));

        erc20_token = new ERC20Mock("wVARA");


        uint256 amount = 100 * (10 ** 18);
        erc20_token.approve(address(treasury), amount);
        treasury.deposit(address(erc20_token), amount);


        uint256[] memory public_inputs = new uint256[](6);
        public_inputs[0] = 3544317610574872;
        public_inputs[1] = 3818006324670434;
        public_inputs[2] = 1609100126983798;
        public_inputs[3] = 2043470627881931;
        public_inputs[4] = 194624568354568;
        public_inputs[5] = 18446744069414584595;

        bytes memory proof = bytes(hex"18d39978105e6371129a8c670c4958719bf0b860646c2dd760a14c6b5aa04b8e1682aec235c07cc291c2bc14670ab30db45b6c6ce53e7d6e42d5d4837a6a0120183d34eb74c7afdf6d88b54e1bde6948e7f566f6cc374e8bec0ab5553e2b95392ecb009497004b9defb864e8756bbfc830dc0e1f505687c9c4779a32f6783943262140c77797264ea54462073603c736a6c78b20a3016f5493f5cf95556ee81e29ed533dc33499c78e45b8c3c36993a6ad812b7073d8f4ca1a61da68b44e28d00cca5e1481a5bf5fea36beae27af01d45bf45ae9d239fd0e03943c7572c4a7bc2a6770a5201926e0d1c6779e580553bc7cfffafd226b0db88be65e8e9f8a77f90ead631a96254c7ad8b6138976435cb6685e7dd5f567290ac6a4e6e4715cdd441418e1ec0c96cca970d2edc68c95b14e42a0bedb073038588c452fcc3ab85c5d1725a1a7880200a962e465e0f9d3f17fc3159f80fbfd30dc098cdc1a99737c44091712fdc9915499cb86525dca25f08198a7b402679d863eb2a02445fad7e28429afaf7c029fe6de81b785f1453e2f44c0c97c0618519c25c955c64156bc4ebe108f6d877fd532555f808b338826e1234c20bb2ccb22da3115fc75d93e41b0b21bd41532aafe2c5ac3ce6cc421cd2c4617aefb685fe0edeaa4938e6dd517820d09da9f3f01d8ede516dac6789e50a13567d2e439eeafdbfa2591a3ddfb128853087aae48a9d53e1d8fb48ee4515b37291704f31cf4d884035920a722325c47d404f63a5ab3833cc17c7117d088197ede501a1d2aa5e26cfbc4946734edf825a80c0bd829d71a6ff5be13ff2c21cb0e3dce66f73f7c30deae6c08738a0b6f231502620c55b44eeb77d256650ba7ade32188a7b72a1758cfc9b0df08e96db5728d2da080f494511bb845c10e66678a76337ebb3dd38980c827543059a159f7fdb62383d97cb2a8b89e16bbefd2111f7d67f0f396e10468e916e85c56b65222294520172052b927228118ade9c2a5345d38831c1ec55bb06534ee94ba43c072f7fa2303ac1d8973c436bb1c7b32bb904bb14c0bf00d8aaf28ff1c7f1f4cf8f7767e105c59c10c4daf99ddc0bcfb3cf4d124613dc9beeee7432d69312f3173edf7d31b1920e827a8ac303e56138695f31ea541b623e6b42cf3fc32635b806dc2f80c1a9c32580fe608a068ce6ad82d81aec14d4ff6e4289716e2d775764554fa24cb2e6766d5885115b9ba39aabcfe166368906efca5c804adecb21f7e84a9ba51b91cac472170ed426ab2407c18a25e5dd9dbdefaceed5249559e537100d9aad4df");


        relayer.submit_merkle_root(public_inputs, proof);


    }


    function testWithdraw() public {
        ContentMessage memory content_message = ContentMessage({
            eth_address: address(treasury),
            vara_address: VARA_ID,
            nonce: 1,
            data: abi.encode(address(erc20_token), address(this), 100)
        });

        VaraMessage memory vara_message = VaraMessage({
            block_number: BLOCK_ID,
            content: content_message,
            proof: bytes("")
        });

        bytes32 messageHash = content_message.hash();
        bytes32[] memory proof = new bytes32[](0);

        message_queue.process_message(BLOCK_ID, content_message, proof);

        vm.expectRevert(abi.encodeWithSelector(IMessageQueue.MessageAlreadyProcessed.selector, messageHash));
        message_queue.process_message(BLOCK_ID, content_message, proof);


    }


}