// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

import {Test} from "forge-std/Test.sol";
import {VerifierTestnet} from "src/VerifierTestnet.sol";

contract VerifierTestnetTest is Test {
    VerifierTestnet public verifier;

    function setUp() public {
        verifier = new VerifierTestnet();
    }

    function test_SafeVerifyProof() public view {
        bytes memory proof = bytes(
            hex"19d4cef1d44499f18661c86024492adda2eafa5ce718d0338c6653bf353f803d1712a538d6b2a7f7c91591ebf9e3ee062cbda5fd77b4d0dd21bf8d61c48002883019289c395cafa1f29cb150a42225b66ee6c5ebf8ea84fe9d615abb8128ac291a34cff8f1ee56dbb4ad25284e4e864db85a809278cd84496a07c58378835f920a424f50ce352f41d7d27cf33e0ae31cc7e8cfa559a67e26a08f16a2f1cdf5ea079d8e9d7b9e668b5bde19664e3bed35af688386c6c93498922db88e4b5a02062b240bbfb6e65ce4f4d06415f0047cab6f4d439d218f6f6ab336458bde735f791eb8a39aef494106da910be2a43ccd7fa598817229370d6b05bfcee248053a4d0af4e6deadba0d9f1a92f87e5e65e57c065d9c23c32cc7c8cb08ce49ead2709b2f198063d7bea04c20cf89261ae42035f3b6661c18b8b516c90cb6cf202537560a3e721a70ab8e5feef90ad13b2b043132b5f9ec1ca0ed8b23e73be996b318c4080120074013cd2c301957177257c7125dacacdedbf6aad203b1ee5781450a211073677b3abc886efdc9d43909754686c174940b1c51585a00d5e0bea93e9ef5223c9930bb93b2f324668beb035ea6fcdaed20495fec6649431d6f9c729f91ec1755551a80cfa077db8175371314b9d7de415db8b760f386baa68c52adefafa81dc93e1a0d9528a83feca29f4970d1ec7e81502059df9c671b457a56ed8fa1f410e04d7549207aa42bb5916787b8003f4ddfadaeb089f1800129a98b464a271f12753b7ad7ec1ca60fc5e03a00522906e5e0ce9998eedc26a538dedde8a12c4e04fb72fa98cd636a56212730d27ecf5eb7feb19cf3dcd1e5c9807d0aa7a6848b1d2f943e87d75a44af766187ba0c6d1d89c83491bc2ecedf513757ab4a0cf10824a1b1163f69aaad7eff13ae5deb9dc528cd1a17c5609ee5c5514158a568bb8503fab5ccac261c52d318dc477142f1e8b3d5cee780a4c092e8832d16e0652985287c554b7689b32baed7cb4532437957e08d7db3d9a33c025f38b5fd3a931dd72e1c83c29385d8528c3374f9634d465005bc4603417620c8cc9849da7f83c2852713aa149e09a3acfa1d6b2d2c40f35c13f71ac3169b5dfe5d25125386f9bf0505bdff26ea8790cec9ab5255b3468e62f13d4378fdfb819495e881b58723d85c0eb889d41f1c98f51551d7c31b0e6f190caecc29ad0f4338763c2b8367ce496c2b04806f7b09eb0f145cc7b23e98c1eb6b7e5da0ee3b2470b10bc4e57a5e43de2ed6794c3eb9effaa1d5482c74ff0b59f95e791dfe5bac8a299607287216577c"
        );

        uint256 blockNumber = 24383731;
        bytes32 merkleRoot = 0x869ef62b91c490f37173a7dfbaacb3fcee64b4225d7e0977435b054efdcb54b2;

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
