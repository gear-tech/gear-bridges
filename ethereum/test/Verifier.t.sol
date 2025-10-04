// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";
import {Verifier} from "src/Verifier.sol";

contract VerifierTest is Test {
    Verifier public verifier;

    function setUp() public {
        verifier = new Verifier();
    }

    function test_SafeVerifyProof() public view {
        bytes memory proof = bytes(
            hex"28791dd7a92916199aa0b46dace659169ea1fbb5b5f335c660e7617f1b4af6d72a06fdd16e966457c7fccad4e577f803d8e2eac2dc0af32c3bb8937be2ce67ac2cba4d4e396e3c1534fec52412b0d96d7a5302e2d49f72c90db5361dbf62f6e21fb1bc6ed551e2941d4c9ad75158301fa93a158113b07d562006760c147f95fc275c0f75cc8dd48bffd8911cede9430594c3adf74307fba6b375be24b00c4d45141eef570d8a75709495c96dfbde489ef3b241df4ca1f76807f6f39461d3ab4622e55343acba861186eff1e81373a729d6f697d01f5b5253d56464d48f8fb8e71e4d7bf10450e685894145360a1a5e4e0b9721be72be0c5085e5ef93aaf8fc5614d2b709dc33f942ac16574c6b5082f60ec09a2856b1d83d60b3c76290ff34c70d916aabb61bb97ff55373dc037ea885256a752a42e4cb279fb182eddd11583c1a6b4522a82c35292ce598fa02da7f890d3eb827476bb8bd1da07bbe5026172602115864c55efd9695b51841775af9aaa56729bc65bc8cceb36e1c82101c3dbc01551bb0d5091cb8e6e8a073a53ea18e5fe062dffff2549741cb820434d0b82a14435f97d2c252b62193bc277e3bb5db4b66b16dde0df5149f33aa343c1cded722858862de3decb64020594ca23c84218b39ba4c94fd1dd31b704a460c5ca2702cedb73d2f2932145bc8595a4ef00a9915817bd66b3c4abc6dd3dac3b884abaf22c5517e2b408339d47957c3179ca4988992a132fd004d75844b8b067ceb3a4e032abe1000757e05f96a8a5b7b58ecf0a009afb5d7c3065f4144da427582af4e1fc026bd0869c78d4f18144b7cf575acdd9e691dbad6625d1716b3a0600a0d732377724242b00d0964099a9f1ec985589f453635acc25a76e514841d56664ad81b7102b5f242fc78a4b18778d6d526caf3425444ad72c12bdc679b4944e0223d2804f45ea8f01eaaf802e1f30d9c3744a3b1441d15eb9086faa528079cba543e0964290e0df79596c9a37e6c3887731bb9a8f9277ad4a154be0d649292ef67ce131cfc5302158a7f5893742cf8190c4ba904502b9924af969c0908ac4d274bae21f2abf18fd93cbd6d959667f4b29f69e53ac864b8c0503d229fddc64b5413770fceb86ceed63bb1407cc134f57a08fd064981fea51e5915cc0972d2566f73db22895b20733d60395575aaaf8fb81894a66c0515c4f50b6ac17a94986eab06f32c4dbfb6c5fb7d772a134f55445448c45f2d64091967861854f412d1c6dbd51e1345662d83cb7cd4a03d66264baa1d938dd5418cb72f35397ca81ab10407f009"
        );

        uint256 blockNumber = 21117627;
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
