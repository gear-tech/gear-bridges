// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {Test} from "forge-std/Test.sol";
import {VerifierMainnet} from "src/VerifierMainnet.sol";

contract VerifierMainnetTest is Test {
    VerifierMainnet public verifier;

    function setUp() public {
        verifier = new VerifierMainnet();
    }

    function test_SafeVerifyProof() public view {
        bytes memory proof = bytes(
            hex"11744d7e4aa34b139f632638e67c15e15d3d6b8c5f3b8b1f5da35186979f93342d2e0a31084631dc3fbab7581cc52123c99f5940cb3268b8d3afd307a7a457560aea7c2e0664b4c66e94299ac71a32c39e379efca9a1224df65086830a38399b1b6dc3d42d99778c93bb25196a7470949a56f22d91a0a75b01ff3c7b426a00e2285b507a391f06c9ff445fc43d102f8d508e03c0d8770ade9643615f250609fd21cde31ff99752542c8788aec88fd55de29c46194fa8e4541159d73631fa5eda2b66d93c4c8a4233cb2aa794a68aa4de2382317fac5737e09fd0566d7181cb5616b615d50783bc640769a93eaaf6f9ba7f37879b3fd9506b4ef1c990dd8c8639249a616d0f74de1fd776b442f7a5ec6ccd6887acbfdb2ab245af302dabb0361430609bb6dc15d6710c284febeb63a8c9e4a45ca1bd0d0c3d9240ebd8a6b97b2229f8f966ba98472bca0e4b5c686994f9f076d2707876d111ad8aa8c571cc60ef088a8c81cd6b2b20afef0b717f45f4c1540e4eacca6db3e1aec48f517c1ac6e413a3f163f7f75168cb5139d01eac74ff41fe3ee29198b7a4b1f86f596a8d7f0c2178910f107b27da7b4e3d241005463e209833827e5e23a1a5497e48511e9b8e1ab032a3b6630ea51ec48106f5caf4cfd7cb8589c6c37476dd6b1f173254af6111bb36f30adf07d8e044330730c9945d0db55984475b17cb7cc5af82c06b610d28aa80d85818b6a94ed5768020f93fecbf6c4d0228c440291c0ca0b4ef725acf1aeb46cfd76ded9f53d18b280b0faed10e76e9892976d14e55975b74dadf8c2b028ff024fa063852ed6026fc298a8b2c56760568ed7c73eaa9d7983df3bea3b907dc4960157423093b8c060427303d39c1bca0002f49ee5dfee002f66e494dba0269096b87b7a6d556065e42652ca84eb721a0558bf41e60ff4d96e0864d099324cdee76dc7bce7b4077a5a4409a172c8d3eeeca3ce7479b8e1a382931d4fc9d1626f4bfac278e4f9771c957028f9c5b8bc48a5ecbff34c975c3018a59d06db024b822223a2e8450f7f2d7343582b31fd79f8589bd3c5051e86b00ca51aacbc200e20a6c8d66e7fd73f49abb781c9a2aeb9bbcb704921b5dc5d0b73d014d3c6e157ab0509e198c92f66ac169549d404d778d0fdfdcfbb952ef3e9e5d9c357d322182a6b4d40bd37aea61c539dac7df355d7d77730c61d349de9bf319d67e8af20139f9366290ff174401b3a697e7137e23dcf551b4261306c9a59b886350a4ff260cc52236a29bed26f0fbe9a4a64f00f5287f4d6dbc1c96af3d80f211620c0a"
        );

        uint256 blockNumber = 30068803;
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
