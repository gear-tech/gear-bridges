// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {Test, console} from "forge-std/Test.sol";
import {Verifier} from "src/Verifier.sol";

contract VerifierTest is Test {
    Verifier public verifier;

    function setUp() public {
        verifier = new Verifier();
    }

    function test_VerifyProof() public view {
        bytes memory proof = bytes(
            hex"065bdc6ece0c2829d5852f32467daa76abcc9a5d2783f4ed8ecb9a46bd5d5ccd132b34cbda20cdbbd9d70d200b62d39ee3bf67176bb0f6fafb9e87f2dd66715b1a9f03d7afc1d745b484017e5e9f14e97c2a03767d979f0cb946213aabee0a0e2ad5e7f5a6bfb81ffbaed13d345f68b55c3f1d50c0c4055dfd5255d9e250bd2f25f37b41bf9f58c3158195b597c3d1c8918101b4203a96edd089afb0692f84fa3013954a166f5b2930c6dc7e5bb0836eaaf745814e12a311c5cf4ca842198d751b8596333f19a2c3c6822bef1f517b47c9e76694cfdecf8db585e0a9f8cd95e11875ec3ce6f158993e82aa85d5f903191bc8f80b9f8ca8be53cd4df570bd6d4008e587e036f3dac45cd3a90a3fa62f9342a01182017935946a1f5dd102327c4717fe80c0f3fd534d05f24ac3602c5a4903bcfe7c89bbfc61a2b7266b891c44df29f282a9fb35adb5148d568672a57dc03790497b09adc12852b1034add68049d10cd21023630c7dc7b4853c5dfdba47e072616a1ad6557df88d148409d3f04ec145ec2612326a0540fc5f5b5b066bcb5c2a0e7c49aa6bcd255484e47fb5999d30f2071550ced2407809d29f3d1bf7441f528c372a06743bc4ed3987abf150e4a0141dfc5d547dc3da6f7af5335d32ac825ce6e1f8924e89eb7516ace1793c36626a09b9fbe03d0dffeaf71979c817013c7bee550c5017b00022abcf9460128ce22756710d9115a33baec4ddc01d3175ce97cdbe7b2b0ebed003863bf72a057f62721da4abcf7c227acde40679911ef3884f8dc9b4999e3cdde876747e686541101c9c6d9ed3d535f2699b67e30f4c33faf8ad9ea0a153907e3e08ca0d4e41012082751beeb1681a059e7224615e51077974fe4d5c87f40cc605140690b52f0fa25d9f2e310123d31420db293d6a67c6b1d2560bb05e0c427deb5e58d9f13c3e812439f1c0749d433eab9ab6a4e0666e4cfabf53c610246d124d7359cc9fcdb69035013290d3e2e22d63229ed852c156a7e29ee8add3c0db8d85ae7f2d1523b152716ab1633be636fb75178b8afcbf4f8af6b22a866cb7e504a1f3ebb199808bc04fa3174af25588139878b395c31a2005fd179b4bd53c78c33ac31930408bf0e13ae9c4971a2a84135cbe07ae70474b5d5156eb1df5630deef0f3a20b8d393610ba3d8fb7b819c5ffdabc2a28352331dbbec1044aae2516223a11eac55be8f8f18e4d840feb92ebb42a0ded285db7f456a8b09709c004e16c4e827e9c7a39bcb2971c76b9f2ea3f54fe85fbb7d51c9112f2cd4b4df8c4b3c8e1ccb889433ee96"
        );

        uint256 blockNumber = 17763872;
        bytes32 merkleRoot = 0x3ca3ea739965faabd22fce079a9ed2cd75681fce46c4b49cee2f376390fba6d4;

        uint256[] memory publicInputs = new uint256[](2);
        publicInputs[0] = uint256(merkleRoot) >> 64;
        publicInputs[1] = ((uint256(merkleRoot) & uint256(type(uint64).max)) << 128)
            | ((blockNumber & uint256(type(uint32).max)) << 96);

        assertTrue(verifier.verifyProof(proof, publicInputs));
    }

    function test_VerifyProofWithWrongNumberOfPublicInputs() public view {
        bytes memory proof = new bytes(0);
        uint256[] memory publicInputs = new uint256[](0);

        assertFalse(verifier.verifyProof(proof, publicInputs));
    }

    function test_VerifyProofWithPublicInputsAreBiggerThanRMod() public view {
        bytes memory proof = new bytes(0);
        uint256[] memory publicInputs = new uint256[](2);
        for (uint256 i = 0; i < publicInputs.length; i++) {
            publicInputs[i] = type(uint256).max;
        }

        assertFalse(verifier.verifyProof(proof, publicInputs));
    }

    function test_VerifyProofWithWrongProofSize() public view {
        bytes memory proof = new bytes(0);
        uint256[] memory publicInputs = new uint256[](2);

        assertFalse(verifier.verifyProof(proof, publicInputs));
    }

    function test_VerifyProofWithWrongProofOpeningsSize() public view {
        bytes memory proof = new bytes(0x3a0);
        for (uint256 i = 0; i < proof.length; i++) {
            proof[i] = 0xff;
        }
        uint256[] memory publicInputs = new uint256[](2);

        assertFalse(verifier.verifyProof(proof, publicInputs));
    }
}
