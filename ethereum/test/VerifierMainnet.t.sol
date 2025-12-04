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
            hex"073a32d4cfea1067b4efc74d2dbe4c623c79dfdd5ff0bbdf3482fe79418a2b0b2c1effaa07c54d235c9e63048f088a9c3124b2e9bd4e7fa046b92292587153d50a5ae7da59373497c8af0a4fa84832d0f0c8203eca4cfe5e9fae0406844874320ecc7d58cf298c0687045fc8ec9f3f8273adbd7b0e6c74626f2e6be94b182d282679b4ca7520e6763df1518e83a954d3134f1e97852891d8f4ac600ce9695b550191de746967452c79bf3914ed58fcc6b875cbe8ea5d7d3b6e9a2f676b3f69f71f86e1e9c8792971a32135693fb89b2dd3d0779aa68cb2b5d18bac91da8d23140530b23f579f3cac10547f50c1ef1804ba208f4b03ca123fe44a26f11073514700fb43e61030b016a72a650c8ec3e77e785d616218f6901142046c33bc5cef742ad0c63f57303bf88a19545dd452fa2b410da2e16ee475e3ff259940ae8e40af29e001d74612df36f2727a372948e4bf23031a138a8ad1b876272e49b5f562c40c2d85d235fa10b6f823899730c8cf18eab83174de6b29614bfcde8e4ddc7e202691a90aae58224c56adfb93675a1e2ad96142dfcb2736308316f03a6ecea2370517b1b4be7bd1d05ca44cfd7528f6472c757b429d1a6123e70e829d5cd3e08614df30111397fbf0627421a66104193b481accc25f9b6a94ef592ecfd79577681ee9e0b09edb472082e81736373011ae368b19d31167b404ef4b34678f349f7221277a48c7d28662a18e8aa81014d4e3e57872156eb6b33bc43cf9fd29a699671d34eabb859d2f8eb457e8dba1f06809f51899db19c3c77cbaa1af535d9880ef210231b85de8f09eb30790d3c9e70090903242330ebf08ebd28d04e1e71623c42c6228886a920900cd3edebb334ed7fd9a10d0c969d3534ca95bc6a739645d03013e7bea3f239d06976c4cff5b06e8ada491f341bfd488cdd2607cf1cfd9afd80ac48e59f33778b9ebcba364d6977b9d72c437fae755bf1882729e3bdf58df3119e5242341b5ba9fc03f121861f6e9b50e446720d0889f47f82155643b9bebbf1a70555390cc8d94c01b919e2eb2a3b56d7485e3ef113584542db62a444abe8909b7f9d7647ae441a97afd4944b385c97649984a33abe7669973c461d81672031e90862f9a0a860916593c8ae2f1ba0697986ce8b11540eeb094c500b492f30a18e8c886ca78a3a9d3a7ba3dad5363f55cd4f52913b97ce28eb667964e19ed8018b6569eb7e94d907d73e356f79efbabebf2ccac11199316d5d9bc5a80e2de18271883dd90bf1fd45d5ee19f2ea23315992a75450c933db779697bcd4014f1f0"
        );

        uint256 blockNumber = 28355670;
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
