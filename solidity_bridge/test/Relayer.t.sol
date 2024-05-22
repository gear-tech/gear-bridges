pragma solidity ^0.8.13;


import {Address} from "@openzeppelin/contracts/utils/Address.sol";


import {Test, console} from "forge-std/Test.sol";
import {Verifier} from "../src/Verifier.sol";
import {Relayer} from "../src/Relayer.sol";

import {Treasury} from "../src/Treasury.sol";
import {ITreasury} from "../src/interfaces/ITreasury.sol";

import {MessageQueue} from "../src/MessageQueue.sol";
import {ProxyContract} from "../src/ProxyContract.sol";

import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";
import {TestHelper} from "./TestHelper.t.sol";


contract RelayerTest is TestHelper {
    using Address for address;

    uint256 private constant BLOCK_ID = 273;
    bytes32 private constant BLOCK_MERKLE_ROOT = bytes32(0xa25559d02a45bf58afd5344964269d38e947a432c1097c342f937a4ad052a683);


    function setUp() public override {
        super.setUp();
    }

    function test_empty() public {
        bytes32 merkleRoot = bytes32(0xb1029042e7087428694e243cb5b777d17a1676d9074debb6fe2c9789c0264418);
        uint256 blockNumber = 274;
        assertEq(0, relayer.getBlockNumber(merkleRoot));
        assertEq(bytes32(0), relayer.getMerkleRoot(blockNumber));
    }

    function test_build_public_inputs() public {
        bytes32 merkleRoot = BLOCK_MERKLE_ROOT;
        uint256 blockNumber = BLOCK_ID;


        uint256[] memory public_inputs = relayer.buildPublicInputs(blockNumber, merkleRoot);

        assertEq(public_inputs[0], 3980403427572212499963242599334442163722879490045996792884);
        assertEq(public_inputs[1], 1166562204472425303272494454897619262805894610326304849920);
    }


    function test_add_block_with_inputs() public {


        bytes memory proof = bytes(hex"203b6d7ee470fd6201aac1d849603241e3303f0ed38c6caeffeafa7708a700f0219f2065a8517c79e6c5dd7f3cf97709fea069f2e30787d283ea75461bcfb7231020f6d4cda614519936afcfd343abd4ec6620c722ca4ac82facdda42526927724e59115798dae55e08fbb386e18d9d843015168b94802845012f7943dd6e6560e90e844f40e7e20d1bbc1221f997cc57308601436354424e3ad38e5060dff630779a7b023f1af6923d9ec2d5f42ee311c387de28e24a5d4e689af858e8ff8b80182ca8d21874a644a26dafe33531d6f626aadd0436ff341ca72c5bad16506580c7e2ab7d32c38097c5ca47fe23bb118a75963b23ad671eff3edae03b30443ad28b05c94bb33b5dda0601a2e448e9bcff356a20aca2fca8548b3aa589d9ab3cf0661bc6e5fc4a2fd9cf752daa21d89c1c68300e0e6611d3461a6cf5b2111de14006cbc8af011601630a2940a972a880adfbe689f2bec6d53ecbda6a1408dece008702afebed1dbcf1be649d794abb58afac334310a248655ddba60e50076a05a206eaa36097d6572598071e178e79675c05ecf48bf64bb1fd19cb3df06c7c6af129bbdac42d8b090938ea97fc22f6cd607a44e168c625bf19254e1c4fe09b6a600b2f423299b72662a65ef56fce78a3ec88ade6ca54848619bf1da88764804b909d6f1e2d3e60e0b52622b64df9d56f5e743628b82c17a688be2b70cb37aef0211f854d5fa134e51a631225c700746d40ef9fdd8c10324949f4b50ab3ab25f5c1352fbaebb8b145be5c2f287899f0547d47254fd47a68ab2bdb4cfc6e9109d7a14d3b2e41225840451765085cd1799c88f270d6356e3a096cbf53a6f1c7838f5036e02246259487f2f340cd0d41ebe2b403e5596361f90c68fadde8aa891e7200b504aa7ff0b5dff127c695b0f7c33b4e1d4e57c03820ed492dc121796e096cc2ec27ee9037b56e0ca44693352ac335b687b757fdfb87136cfde7cf1865d54b9066ba8e5e9bdbf0fbdab7b1a02840ef1c415a51e74d9ef0812d9bd67e3a413b818d7fbab3649c5a5d8705d896f0a1a3b140d938486b99830c171108a862b0fa72e0943712e094e05cf1b5d50ee5422962bde5d533a4d7cc7ee7b2824148e71d81a3a3a8ec8091f8b52bc11ffe5189516441a01815250defe8d1e1e4150c4852c0ac274e45671a86b35be16b26f69bb60945f40e0caca8efbb998a268cf9db32927fd92d29a36c1b33d7bfe0540580c7a6628bcd28ead55135d8ad785b6e0424d1e870edf3353bad820bf5c7fa6e4fda335793fde58de57e062990001a8a30e07");

        uint256[] memory calculated_public_inputs = relayer.buildPublicInputs(BLOCK_ID, BLOCK_MERKLE_ROOT);
        assertEq(calculated_public_inputs[0], 3980403427572212499963242599334442163722879490045996792884);
        assertEq(calculated_public_inputs[1], 1166562204472425303272494454897619262805894610326304849920);


        relayer.submitMerkleRoot(BLOCK_ID, BLOCK_MERKLE_ROOT, proof);
    }


}