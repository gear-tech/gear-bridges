pragma solidity ^0.8.24;

import {Test, console} from "forge-std/Test.sol";
import {Prover} from "../src/Prover.sol";

contract ProoverTest is Test {

    Prover public prover;

    uint256 private constant BLOCK_ID = 274;
    uint256 private constant P = 2**64 - 2**32 + 1;
    uint256 private constant MASK_52BITS = (2**52) - 1;


    function setUp() public {
        prover = new Prover();
    }

    function getMerkleRootFromPublicInput(uint256[] memory public_inputs) private returns(bytes32) {
        //uint256 blockId = public_inputs[5] - P;

        uint256 ret=uint256(public_inputs[4] & MASK_52BITS);
        for(uint256 i = 4 ; i > 0; i --) {
            ret <<= 52;
            ret |= (public_inputs[i-1] & MASK_52BITS);
        }

        return bytes32(ret);
    }

    function getPublicInputsFromMerkleRoot(bytes32 merkle_root) private returns(uint256[] memory public_inputs) {
        uint256 root = uint256(merkle_root);
        public_inputs = new uint256[](5);
        for(uint256 i = 0; i < 5; i++ ){
            public_inputs[i] = (root & MASK_52BITS);
            root >>= 52;
        }
    }


        //[2787997088524558, 914341688072726, 3440393019007615, 3418656939423883, 276187037400784]
        //0xFB30CE417ED0C2540187BB88BC3904EBC9227F33F96C3855A169E7AB2818A90E
    function test_merkle_root_from_public() public {
        uint256[] memory public_inputs=new uint256[](6);
        public_inputs[0]=2787997088524558;
        public_inputs[1]=914341688072726;
        public_inputs[2]=3440393019007615;
        public_inputs[3]=3418656939423883;
        public_inputs[4]=276187037400784;

        bytes32 merkle_root = getMerkleRootFromPublicInput(public_inputs);

        console.logBytes32( merkle_root);
        assertEq(merkle_root, bytes32(0xFB30CE417ED0C2540187BB88BC3904EBC9227F33F96C3855A169E7AB2818A90E));
    }

    function test_public_from_merke_root() public {
        uint256[] memory public_inputs=new uint256[](6);
        public_inputs[0]=2787997088524558;
        public_inputs[1]=914341688072726;
        public_inputs[2]=3440393019007615;
        public_inputs[3]=3418656939423883;
        public_inputs[4]=276187037400784;

        uint256[] memory inputs = getPublicInputsFromMerkleRoot(bytes32(0xFB30CE417ED0C2540187BB88BC3904EBC9227F33F96C3855A169E7AB2818A90E));

        for(uint256 i = 0 ; i < 5 ; i ++ ){
            assertEq(inputs[i], public_inputs[i] );
        }
    }

    function test_block_proof() public {
        bytes memory proof = bytes(hex"18d39978105e6371129a8c670c4958719bf0b860646c2dd760a14c6b5aa04b8e1682aec235c07cc291c2bc14670ab30db45b6c6ce53e7d6e42d5d4837a6a0120183d34eb74c7afdf6d88b54e1bde6948e7f566f6cc374e8bec0ab5553e2b95392ecb009497004b9defb864e8756bbfc830dc0e1f505687c9c4779a32f6783943262140c77797264ea54462073603c736a6c78b20a3016f5493f5cf95556ee81e29ed533dc33499c78e45b8c3c36993a6ad812b7073d8f4ca1a61da68b44e28d00cca5e1481a5bf5fea36beae27af01d45bf45ae9d239fd0e03943c7572c4a7bc2a6770a5201926e0d1c6779e580553bc7cfffafd226b0db88be65e8e9f8a77f90ead631a96254c7ad8b6138976435cb6685e7dd5f567290ac6a4e6e4715cdd441418e1ec0c96cca970d2edc68c95b14e42a0bedb073038588c452fcc3ab85c5d1725a1a7880200a962e465e0f9d3f17fc3159f80fbfd30dc098cdc1a99737c44091712fdc9915499cb86525dca25f08198a7b402679d863eb2a02445fad7e28429afaf7c029fe6de81b785f1453e2f44c0c97c0618519c25c955c64156bc4ebe108f6d877fd532555f808b338826e1234c20bb2ccb22da3115fc75d93e41b0b21bd41532aafe2c5ac3ce6cc421cd2c4617aefb685fe0edeaa4938e6dd517820d09da9f3f01d8ede516dac6789e50a13567d2e439eeafdbfa2591a3ddfb128853087aae48a9d53e1d8fb48ee4515b37291704f31cf4d884035920a722325c47d404f63a5ab3833cc17c7117d088197ede501a1d2aa5e26cfbc4946734edf825a80c0bd829d71a6ff5be13ff2c21cb0e3dce66f73f7c30deae6c08738a0b6f231502620c55b44eeb77d256650ba7ade32188a7b72a1758cfc9b0df08e96db5728d2da080f494511bb845c10e66678a76337ebb3dd38980c827543059a159f7fdb62383d97cb2a8b89e16bbefd2111f7d67f0f396e10468e916e85c56b65222294520172052b927228118ade9c2a5345d38831c1ec55bb06534ee94ba43c072f7fa2303ac1d8973c436bb1c7b32bb904bb14c0bf00d8aaf28ff1c7f1f4cf8f7767e105c59c10c4daf99ddc0bcfb3cf4d124613dc9beeee7432d69312f3173edf7d31b1920e827a8ac303e56138695f31ea541b623e6b42cf3fc32635b806dc2f80c1a9c32580fe608a068ce6ad82d81aec14d4ff6e4289716e2d775764554fa24cb2e6766d5885115b9ba39aabcfe166368906efca5c804adecb21f7e84a9ba51b91cac472170ed426ab2407c18a25e5dd9dbdefaceed5249559e537100d9aad4df");
        uint256[] memory public_inputs=new uint256[](6);

        uint256 blockId = 18446744069414584595 - P;
        assertEq( blockId, BLOCK_ID);


        public_inputs[0]=3544317610574872;
        public_inputs[1]=3818006324670434;
        public_inputs[2]=1609100126983798;
        public_inputs[3]=2043470627881931;
        public_inputs[4]=194624568354568;
        public_inputs[5]=18446744069414584595;

        assertEq(prover.verifyProof(proof, public_inputs), true);

        bytes32 merkle_root = getMerkleRootFromPublicInput(public_inputs);
        console.logBytes32(merkle_root);

        uint256[] memory inputs = getPublicInputsFromMerkleRoot(merkle_root);
        
        for(uint256 i = 0 ; i < 5 ; i ++ ){
            console.log(inputs[i], public_inputs[i]);
            assertEq(inputs[i], public_inputs[i] );
        }
        assertEq(merkle_root, getMerkleRootFromPublicInput(inputs));
        console.logBytes32(merkle_root);



    }
}


