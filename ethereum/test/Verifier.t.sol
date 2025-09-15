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
            hex"26140c26a1a6fb699120925a50f15c1c79dbeebf44379156ff6ce3bd9599d0941c3554daa3e60fef8830de143353eff247697b1ebbe57a48a83332a70cdd60fc1036c6be9aa013a3030f0bf37234f998e48c5bdda440c86708fbc4a17dc55fd428231c63d0f1262893d4bef51a1c74b9cd51bd4196006111e4c43b350ef5161427d23d369140c2663f74dafcf6aa0d97d218bfbe7aaa2fb8d6137ee2e713158128d897dc9e8ef0d7405d75720ca088595ac81514c28686914a736b0ccaeaf7f31ca80eed7c8f6edb329fa8d48027b002591cdcc63681eea50086e9535c48ea640971cb7a7475cf9caeca23c2b193105f0e71e0ae582f60cae07f66a53b8eb6a316c667afe0acb7fab6db81c9c04f67e443c3d8b7fbb68bdf11e1a1def422e0461010c9364ca7cc7037c8e744d55fed96ce6ebc1a917211830e3ff9f05a17beb01dabb94e05d31eb9eac39551158947d196b7906eb60276a4a0d2445597281e2e14bd6208760d1594fcf065c0e46c906ef1f25ced360d51d15243b1a35f7cdfb0295b3cacd021b4665ad31b9a57742ba477253b072036f970f3d80538a515a0cd0c782a2761a59c605189a0dbb96ccf10c1ea8b49f363dbfabd7f56dfd130e1420314d63a2d19536e0e5b709e85bc6e12308d0ced91c303168cbda1495e6a037918713685bb0a63a57c68adc21382ba80104b848c881be3c3b510d5804b7aa417242933e967991ff67b14e3ef00d5804ca4760cd5924f2423bfa841f16a4aeed41a8af9f3d56cb863bef0175fc3cf205b25d60441407c7daa48be3e04aaf5143206a063eee122c2a86d369714371d5f5332ed1f338d2301f9ea7a2f152abf58022ab9b12d5549e0690d5acb6f93a23c7c75429d81ca8b98fb7c351ccccf998e1c233453b3ef7c427d5626a2582e9acdb0952488af60203d88a687c2ca09b1b41111ed6ef0df62a2af41547ac31d830295aead914f1c2dbb8f5e44072ec020ccf32c999b0b3cac6e6bc836de6f69b33c159883ada98f3f1ba244e94e03fbba47112088ecf3860eb2bde3856af376001bbf1a33112ae839e84619b15237e221a38e2852914bc38ea64592e0205b333d1da0b29d816f579848e5d4e4ebd232690c602e97a09fb1b2b8bab8fb7cbce719acdf3082307aba817b617f2b76188ec7934102cc43223f49df8da2abd1aa90e6ccec654d65d285c690d2a904bd920bc0009f1719b2cdde67cf4c9b8c4f4d8e4690c36d46a98cbc7b3c8d96be27b0a23339db0979f8dd8fec45848b4fad423f07e0374af56a31e9f9fe2d3ca588d0d0840493"
        );

        uint256 blockNumber = 20541748;
        bytes32 merkleRoot = 0x876e8e0111f19c08b9ab0f9a4a088711efb2858a415556b31809324c7b75b373;

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
