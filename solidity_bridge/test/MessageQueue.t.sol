pragma solidity ^0.8.13;


import {Address} from "@openzeppelin/contracts/utils/Address.sol";


import {Test, console} from "forge-std/Test.sol";
import {Verifier} from "../src/mocks/VerifierMock.sol";
import {Relayer} from "../src/Relayer.sol";
import {IRelayer} from "../src/interfaces/IRelayer.sol";

import {Treasury} from "../src/Treasury.sol";
import {ITreasury, Packer, WithdrawMessage} from "../src/interfaces/ITreasury.sol";

import {MessageQueue} from "../src/MessageQueue.sol";
import {IMessageQueue, VaraMessage, Hasher} from "../src/interfaces/IMessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";
import {Constants} from "../src/libraries/Constants.sol";

import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";


contract MessageQueueTest is Test {
    Relayer public relayer;
    Verifier public verifier;
    Treasury public treasury;
    MessageQueue public message_queue;
    using Address for address;
    using Hasher for VaraMessage;
    using Packer for WithdrawMessage;

    ERC20Mock public erc20_token;

    uint256 private constant BLOCK_ID = 273;
    bytes32 private constant BLOCK_MERKLE_ROOT = bytes32(0xa25559d02a45bf58afd5344964269d38e947a432c1097c342f937a4ad052a683);

    bytes32 private constant VARA_ADDRESS_3 = bytes32(0x0303030303030303030303030303030303030303030303030303030303030303);
    address private constant ETH_ADDRESS_3 = address(0x0303030303030303030303030303030303030303);

    bytes32 private constant VARA_ADDRESS_7 = bytes32(0x0707070707070707070707070707070707070707070707070707070707070707);
    address private constant ETH_ADDRESS_5 = address(0x0505050505050505050505050505050505050505);


    function setUp() public {
        Verifier _verifier = new Verifier();
        Relayer _relayer = new Relayer();
        Treasury _treasury = new Treasury();
        MessageQueue _message_queue = new MessageQueue();

        ProxyContract _relayer_proxy = new ProxyContract(address(_relayer), abi.encodeWithSignature("initialize(address)", address(_verifier)));

        ProxyContract _message_queue_proxy = new ProxyContract(address(_message_queue), abi.encodeWithSignature("initialize(address)", address(_relayer_proxy)));
        ProxyContract _treasury_proxy = new ProxyContract(address(_treasury), abi.encodeWithSignature("initialize(address)", address(_message_queue_proxy)));

        relayer = Relayer(address(_relayer_proxy));
        treasury = Treasury(address(_treasury_proxy));
        message_queue = MessageQueue(address(_message_queue_proxy));
        verifier = Verifier(address(_verifier));

        erc20_token = new ERC20Mock("wVARA");

        uint256 amount = 100 * (10 ** 18);
        erc20_token.approve(address(treasury), amount);
        treasury.deposit(address(erc20_token), amount, VARA_ADDRESS_3);

        bytes memory proof = bytes(hex"203b6d7ee470fd6201aac1d849603241e3303f0ed38c6caeffeafa7708a700f0219f2065a8517c79e6c5dd7f3cf97709fea069f2e30787d283ea75461bcfb7231020f6d4cda614519936afcfd343abd4ec6620c722ca4ac82facdda42526927724e59115798dae55e08fbb386e18d9d843015168b94802845012f7943dd6e6560e90e844f40e7e20d1bbc1221f997cc57308601436354424e3ad38e5060dff630779a7b023f1af6923d9ec2d5f42ee311c387de28e24a5d4e689af858e8ff8b80182ca8d21874a644a26dafe33531d6f626aadd0436ff341ca72c5bad16506580c7e2ab7d32c38097c5ca47fe23bb118a75963b23ad671eff3edae03b30443ad28b05c94bb33b5dda0601a2e448e9bcff356a20aca2fca8548b3aa589d9ab3cf0661bc6e5fc4a2fd9cf752daa21d89c1c68300e0e6611d3461a6cf5b2111de14006cbc8af011601630a2940a972a880adfbe689f2bec6d53ecbda6a1408dece008702afebed1dbcf1be649d794abb58afac334310a248655ddba60e50076a05a206eaa36097d6572598071e178e79675c05ecf48bf64bb1fd19cb3df06c7c6af129bbdac42d8b090938ea97fc22f6cd607a44e168c625bf19254e1c4fe09b6a600b2f423299b72662a65ef56fce78a3ec88ade6ca54848619bf1da88764804b909d6f1e2d3e60e0b52622b64df9d56f5e743628b82c17a688be2b70cb37aef0211f854d5fa134e51a631225c700746d40ef9fdd8c10324949f4b50ab3ab25f5c1352fbaebb8b145be5c2f287899f0547d47254fd47a68ab2bdb4cfc6e9109d7a14d3b2e41225840451765085cd1799c88f270d6356e3a096cbf53a6f1c7838f5036e02246259487f2f340cd0d41ebe2b403e5596361f90c68fadde8aa891e7200b504aa7ff0b5dff127c695b0f7c33b4e1d4e57c03820ed492dc121796e096cc2ec27ee9037b56e0ca44693352ac335b687b757fdfb87136cfde7cf1865d54b9066ba8e5e9bdbf0fbdab7b1a02840ef1c415a51e74d9ef0812d9bd67e3a413b818d7fbab3649c5a5d8705d896f0a1a3b140d938486b99830c171108a862b0fa72e0943712e094e05cf1b5d50ee5422962bde5d533a4d7cc7ee7b2824148e71d81a3a3a8ec8091f8b52bc11ffe5189516441a01815250defe8d1e1e4150c4852c0ac274e45671a86b35be16b26f69bb60945f40e0caca8efbb998a268cf9db32927fd92d29a36c1b33d7bfe0540580c7a6628bcd28ead55135d8ad785b6e0424d1e870edf3353bad820bf5c7fa6e4fda335793fde58de57e062990001a8a30e07");

        relayer.submitMerkleRoot(BLOCK_ID, BLOCK_MERKLE_ROOT, proof);
    }



    function test_calculate_root_buffer() public {
        uint8[98] memory msgt = [3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3];
        bytes memory m = new bytes(msgt.length);
        for (uint i = 0; i < m.length; i ++) {
            m[i] = bytes1(msgt[i]);
        }

        bytes32 messageHash = keccak256(m);
        console.logBytes32(messageHash);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = bytes32(0x127e5bcfb1c26b19c1dc1a29182cd1d978e5900a8483cd33c656fdc65b87dcb8);

        bytes32 root = message_queue.calculateMerkleRoot(proof, messageHash, 3, 2);

        assertEq(root, bytes32(0x9f88b3c5da39e8d08c9ce048d51e9be248a1c07b2abc986ea5522d2f8e662044));
    }


    function test_calculate_root_buffer_2() public {

        uint8[86] memory msgt = [3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3];
        bytes memory m = new bytes(msgt.length);
        for (uint i = 0; i < m.length; i ++) {
            m[i] = bytes1(msgt[i]);
        }

        bytes32 messageHash = keccak256(m);
        console.logBytes32(messageHash);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = bytes32(0x4460e63f13779139d1f836f7f72c36b62340ffe74beceeea0f2c08a0195a151e);
        console.logBytes32(proof[0]);

        bytes32 root = message_queue.calculateMerkleRoot(proof, messageHash, 3, 2);

        assertEq(root, bytes32(0xbd0053b78e8ecfb691c483db70d9792b0ff1b9956dc78967af2c4d4f1872f206));
    }


    function test_calculate_root_buffer_3() public {
        uint8[98] memory msgt = [4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3];
        bytes memory m = new bytes(msgt.length);
        for (uint i = 0; i < m.length; i ++) {
            m[i] = bytes1(msgt[i]);
        }

        bytes32 messageHash = keccak256(m);
        console.logBytes32(messageHash);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = bytes32(0x127e5bcfb1c26b19c1dc1a29182cd1d978e5900a8483cd33c656fdc65b87dcb8);
        console.logBytes32(proof[0]);

        bytes32 root = message_queue.calculateMerkleRoot(proof, messageHash, 3, 2);

        assertEq(root, bytes32(0x7188ce46fd6dc24003be8667cd73ca4a4cef97687b21343020681d2e192f5fcc));
    }

    function test_calculate_root() public {
        uint8[2] memory msgt = [3, 3];
        bytes memory m = new bytes(msgt.length);
        for (uint i = 0; i < m.length; i ++) {
            m[i] = bytes1(msgt[i]);
        }

        bytes memory payload = abi.encodePacked(m);

        VaraMessage memory content_message = VaraMessage({
            sender: VARA_ADDRESS_3,
            receiver: ETH_ADDRESS_3,
            nonce: 3,
            data: payload
        });

        bytes memory ms = abi.encodePacked(content_message.sender, content_message.receiver, content_message.nonce, content_message.data);


        bytes32 expectedMessageHash = keccak256(ms);
        
        bytes32 msg_hash = content_message.hash();
        assertEq(expectedMessageHash, msg_hash);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = bytes32(0x4460e63f13779139d1f836f7f72c36b62340ffe74beceeea0f2c08a0195a151e);

        bytes32 root = message_queue.calculateMerkleRoot(proof, expectedMessageHash, 3, 2);

        assertEq(root, bytes32(0xbd0053b78e8ecfb691c483db70d9792b0ff1b9956dc78967af2c4d4f1872f206));
    }

    function test_calculate_root_buffer_leaf_2() public {
        uint8[98] memory msgt = [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2];

        bytes memory ms = new bytes(msgt.length);
        for (uint i = 0; i < ms.length; i ++) {
            ms[i] = bytes1(msgt[i]);
        }

        bytes32 messageHash = keccak256(ms);
        console.logBytes32(messageHash);

        bytes32[] memory proof = new bytes32[](7);
        proof[0] = bytes32(0xac9f1d13ebef420edd0101b06f534ec2495ca41af6c23cf14bc94f67bae8dfe1);
        proof[1] = bytes32(0x30cdfaedf81fed4b4564ef0e8c04c56d3481e0121501c2dcc12288e01f3ceb94);
        proof[2] = bytes32(0xf87bc57ba7962a2b733f78df0e777ca31499b78c4d6f64c6d49ab0fd1dc60f44);
        proof[3] = bytes32(0xed0dcf662c10b0827133e6e99e415b0d97da1a92ce69eb717838d55cc9067c49);
        proof[4] = bytes32(0x2387406c963403e53d56621d1cef73b80089994ee4c5866ae2d21eaa9fcdfe01);
        proof[5] = bytes32(0x08ab6b1030ad30cece656ac2638a8aed651bd759a6486241a293610f84927f52);
        proof[6] = bytes32(0xe7e9ede5fe38231d6c068bc8f5d95b76eed9b255f9b892f77c4f640cc86514ac);

        bytes32 root = message_queue.calculateMerkleRoot(proof, messageHash, 101, 2);

        assertEq(root, bytes32(0xbd18567f3cd28d09dc4f8b0f367415dc19f0e32d47424015eaf22103a4bf4cb3));

    }

    function test_calculate_root_buffer_leaf_3() public {
        uint8[98] memory msgt = [3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 3];

        bytes memory ms = new bytes(msgt.length);
        for (uint i = 0; i < ms.length; i ++) {
            ms[i] = bytes1(msgt[i]);
        }

        bytes32 messageHash = keccak256(ms);
        console.logBytes32(messageHash);

        assertEq(messageHash, bytes32(0xac9f1d13ebef420edd0101b06f534ec2495ca41af6c23cf14bc94f67bae8dfe1));

        bytes32[] memory proof = new bytes32[](7);
        proof[0] = bytes32(0x57caf83a5d10cdf3f3a28cdc6426da6a94ce5c2b966a8d08f948470358be53a8);
        proof[1] = bytes32(0x30cdfaedf81fed4b4564ef0e8c04c56d3481e0121501c2dcc12288e01f3ceb94);
        proof[2] = bytes32(0xf87bc57ba7962a2b733f78df0e777ca31499b78c4d6f64c6d49ab0fd1dc60f44);
        proof[3] = bytes32(0xed0dcf662c10b0827133e6e99e415b0d97da1a92ce69eb717838d55cc9067c49);
        proof[4] = bytes32(0x2387406c963403e53d56621d1cef73b80089994ee4c5866ae2d21eaa9fcdfe01);
        proof[5] = bytes32(0x08ab6b1030ad30cece656ac2638a8aed651bd759a6486241a293610f84927f52);
        proof[6] = bytes32(0xe7e9ede5fe38231d6c068bc8f5d95b76eed9b255f9b892f77c4f640cc86514ac);

        bytes32 root = message_queue.calculateMerkleRoot(proof, messageHash, 101, 3);

        assertEq(root, bytes32(0xbd18567f3cd28d09dc4f8b0f367415dc19f0e32d47424015eaf22103a4bf4cb3));

    }

    function test_calculate_root_buffer_leaf_100() public {
        uint8[3] memory msgt = [3, 3, 3];
        bytes memory m = new bytes(msgt.length);
        for (uint i = 0; i < m.length; i ++) {
            m[i] = bytes1(msgt[i]);
        }

        bytes memory payload = abi.encodePacked(m);


        VaraMessage memory content_message = VaraMessage({
            sender: VARA_ADDRESS_7,
            receiver: ETH_ADDRESS_5,
            nonce: 10,
            data: payload
        });


        bytes memory ms = abi.encodePacked(content_message.sender, content_message.receiver, content_message.nonce, content_message.data);

        bytes32 expectedMessageHash = keccak256(ms);
        
        assertEq(expectedMessageHash, bytes32(0xcee28748a98c81d3eb24f23af4876c8d71c75efc61416bfd2bb018390b138794));

        bytes32[] memory proof = new bytes32[](3);
        proof[0] = bytes32(0x69b655dccf32e0c3e4d4f427875a09b8cde36a2e6d1b980a8b1f8b134425652f);
        proof[1] = bytes32(0x6d6e07bcb08ba34a789918ab09f0a8aabd1c42a1e7b8625448dab3ed03a02b59);
        proof[2] = bytes32(0xbdfbb5c1b5550cf03c9819c027ee7d51d3153d372968cdfae6f01d261cb6877b);

        bytes32 root = message_queue.calculateMerkleRoot(proof, expectedMessageHash, 101, 100);

        assertEq(root, bytes32(0x8db8d383e63f1ff7bbd1b35d7d1f240f6fce68aa12e60cd3a446021f8cd04226));
    }

    function test_submit_transaction() public {
        WithdrawMessage memory withdraw_msg = WithdrawMessage({
            receiver: ETH_ADDRESS_3,
            token: address(erc20_token),
            amount: 10 * (10 ** 18)
        });


        VaraMessage memory vara_message = VaraMessage({
            sender: VARA_ADDRESS_7,
            receiver: address(treasury),
            nonce: 10,
            data: withdraw_msg.pack()
        });

        bytes32 msg_hash = vara_message.hash();
        bytes32[] memory proof = new bytes32[](3);

        proof[0] = bytes32(0x69b655dccf32e0c3e4d4f427875a09b8cde36a2e6d1b980a8b1f8b134425652f);
        proof[1] = bytes32(0x6d6e07bcb08ba34a789918ab09f0a8aabd1c42a1e7b8625448dab3ed03a02b59);
        proof[2] = bytes32(0xbdfbb5c1b5550cf03c9819c027ee7d51d3153d372968cdfae6f01d261cb6877b);

        bytes32 calculatedRoot = message_queue.calculateMerkleRoot(proof, msg_hash, 101, 100);

        assertEq(calculatedRoot, bytes32(0x25059660e16a5d756dbb2bbfba28886149e02593e28d64e9256f6b93d5351aaa));

        bytes memory block_proof = bytes(hex"00");

        vm.expectEmit(true, true, false, false);
        emit IRelayer.MerkleRoot(1234, calculatedRoot);

        relayer.submitMerkleRoot(1234, calculatedRoot, block_proof);

        message_queue.processMessage(1234, 101, 100, vara_message, proof);

        assertEq(erc20_token.balanceOf(ETH_ADDRESS_3), 10 * (10 ** 18));
    }
}