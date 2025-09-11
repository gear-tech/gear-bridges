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
            hex"0816c67da05b6789f2d939080d684525819ee54b59637f44a14dcf928628cd762a78827818258e41c071587888cd88d3a90821f868c285dacb89a9ae1a0914b2161bc40c36c31be174641693e9861162c97fdda04c3683b00db149ed47f3cca228d581a86bdbc5ac50dba000a99757b1b0229d244732eefc987c70555e48951f20d57882631d18510c20bf4ec62aee72ae66abcdb8fa9338f81200981f0f8b360eca3a274c697536cc461e11e8c2ec38bdee590698d195c92daf77f4c23053a40faacfe76460b5ef47e9b1ba9b512b82dff49c1d4c3973e4aa74ae623cadc71b17dfa2e21619c0faf6909c70116dd9082087e52bd0403e759c9fd1a8d4652d89100f5f292861fdf354c84867c20fde183b90ea97d9801e0306a5bd5b7c27d77f01132c82dc614061d9154496123dd5b8f324d63e3362962637676d6f513da75d2f7bc847fe6815e476a2994b95b1d7d381d4c5efd650d9e683d21eef1a6d5b7e19b7811aa202d1dfb16591549bb130044d80574d5adc0a9f30235a093ca3db2d18b94dae36abd41ffcede88e40e0bcccfef26fa201271ff2d553e25ff0c36b7d00bc377d9781e1a7922393464bee9f93c1c3a6cec31c391b64c8e720146db1722c6dd2e782789bea321366b2a5f2f1aec50ce482ac40c167df797c52051f1f1b2a0bddc30dd28306f02d072ee61fb18f404e7ff1c64254adaa1d5d87ef6062420b74a058b3ee558f43a6872417c1068ca0f4781f32814382a838b095ab7dca6b2e117eab59305e2424318921906c1de41c22c7cef25d8d289254b842887ab96500cc8d8d775fe5571103bb4b322e49373d9e05debfd739d0d7b0c378a23a2f88176cc5e5285035162c94cea44486c586db851da18633b3b26af97e8aaab74a6a08f397a630fa0259e3e3e52490510d3cdeff690f9daa88f860430b64696d7054171d0e5722520970a3a149c317babff31c3ce3fe58c925b71c4ed4db161624621fad9cde17b3f0e6985c5c25d3a3c2ff48f9133902fa91c1753666c310b493db122faf3f7a56cffb1519ab50f41df90841fb8afffc89347edf15b2c650fc1a7622710e9a35770a539b84c4ed2e9b6dfad4feddf833a6f85909a093866f62285015b2b822242175e72a7d124d1a833bf44a5b3b1bf493d9a017bd0968856d1aec2c85fffba6443c0bfa4be1c9a19a097918116c0729479b0cfa8af95f8ba467a0260e0fe3bb7fa5385dd12911c0bde29fe42e5156cc0cadfd87d420717928bb3d2e7ac508f789b5af26e7a255101941f14f67c30114121843b483ebc3dae94cb9"
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
