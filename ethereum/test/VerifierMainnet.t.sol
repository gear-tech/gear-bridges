// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {VerifierMainnet} from "src/VerifierMainnet.sol";

contract VerifierMainnetTest is Test {
    VerifierMainnet public verifier;

    function setUp() public {
        verifier = new VerifierMainnet();
    }

    function test_SafeVerifyProof() public view {
        bytes memory proof = bytes(
            hex"0af4a4ac6605e6eac5a3c0984d2d2f9dbe842dd278c5ed33b7120973fe68d0be271d99890c1e68fb3a910148c472f2d1b7a6b5c42f4712e4607396087b2a9f9d0b23ce865607022c155e44f4b75e39883396b012e3b03e441d16db9a7016999f08a4eb22a004bd3b52acc728f16c79ec1e2bc118d134f193db0f16bae35113591706210d7f240a9cd297a6ca4bf89e678921f37993d4b6d3f4ee6ab0b7ac3c472be06d10ddea6d5681111c7ffdaa6fffe34c39209ca65292bb946a46524d426908ce1b0a3720dfda98c58e9b2a1fbf1fd5cda3baf7bd34fd6d28eef10fafce9816909e9c0c4423ccbc0ce6972a79d6854f2e7cc3054eeee5d391a9e381af845404caa45875ab47624a65d5f7a0f109a94b3824d333eb3511c8a986f7cd2f84c5057dc7eec302592cdfcb9ca541d4cd6a1bd3c0848661332997199aed8dc261361455f0381da344608af6469731c5c68bad4569936fd3f02a85808714122b8bbb0e5ae58308c9153af4f4a79043eb2913ec47c015bc2476605f42ec056f4040ff24ab103440a14d83384b37bad7ef18cf06f3d971f7a8369790797003e1bdab541b1ce18cbe57ab54a917ea9272fc97a42a19dbfb7cd9eb3414afc1e8170b6c83100fa6913ff089d0a1ddca0d7ed37c5ace917fc6d1207869c2832ed67be913570a8a9d10d2257a6a9bc5109914ae2cfe4786c0b7bb376cd078e6cbb5f19424ca1ef29faf978c6a114876f38685ae44f3ddad7f5511081b824d5fe5ca8bdc529a24bc9f9ca662d6da29e5ce78513b3d63242d9832db7517f9fc7b437262bd598d2dd635cf2c7a2870847aa003b20625e9e20f9c932144a3c254ba01f0ca2864881ecbb468508bcde16d66de1bb939855c3ce4f23a6f9d847c7e6b996eaa0e27d60fc07eb96821bcb094ec35c8b3d78176bf6ea873c381c55190a6575277e003151f000fbd5ffb0d9fa101951becbd3e42f7995406b18f9daa4799d3cbae1c56e52e2b826f25714d332d7f56e3cab381c440e27a43ad3efef48c722995e170d0de18b44c499493dea0be4c0799cddda941bda377e31702bf65e3dc7e774440385d2048ab23bd69343248bd1d9aa26f55230a37749a243593fd94ebbe0e5b77333a258edcc622e07ad694ced71f9c7bf16ee4fb2e160f9b59cd8381504a2666e8d601e057f00d09a08f224cbc777f051b275b82fc9d0d487bb717c15ebfedf631cd1118262e189ad4e065944a221dda2832e5702f8074506f8561a334aea9db6b62242284fb2d18bfd578681f93c427619c2d9ad45cec3fe1bb00e05be5916a2373"
        );

        uint256 blockNumber = 27118808;
        bytes32 merkleRoot = 0x0000000000000000000000000000000000000000000000000000000000000000;

        uint256[] memory publicInputs = new uint256[](2);
        publicInputs[0] = uint256(merkleRoot) >> 64;
        publicInputs[1] = ((uint256(merkleRoot) & uint256(type(uint64).max)) << 128)
            | ((blockNumber & uint256(type(uint32).max)) << 96);

        assertTrue(verifier.safeVerifyProof(proof, publicInputs));
    }

    function test_SafeVerifyProofWithWrongNumberOfPublicInputs() public view {
        bytes memory proof = new bytes(0);
        uint256[] memory publicInputs = new uint256[](0);

        assertFalse(verifier.safeVerifyProof(proof, publicInputs));
    }

    function test_SafeVerifyProofWithPublicInputsAreBiggerThanRMod() public view {
        bytes memory proof = new bytes(0);
        uint256[] memory publicInputs = new uint256[](2);
        for (uint256 i = 0; i < publicInputs.length; i++) {
            publicInputs[i] = type(uint256).max;
        }

        assertFalse(verifier.safeVerifyProof(proof, publicInputs));
    }

    function test_SafeVerifyProofWithWrongProofSize() public view {
        bytes memory proof = new bytes(0);
        uint256[] memory publicInputs = new uint256[](2);

        assertFalse(verifier.safeVerifyProof(proof, publicInputs));
    }

    function test_SafeVerifyProofWithWrongProofOpeningsSize() public view {
        bytes memory proof = new bytes(0x3a0);
        for (uint256 i = 0; i < proof.length; i++) {
            proof[i] = 0xff;
        }
        uint256[] memory publicInputs = new uint256[](2);

        assertFalse(verifier.safeVerifyProof(proof, publicInputs));
    }
}
