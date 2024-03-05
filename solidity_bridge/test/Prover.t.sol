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
        bytes memory proof = bytes(hex"005fd91a2a1b6ccd1a858b5dd8c8297b9e5e26abbfc076de991c1b15823acd002392495fdd316fe43ee8a68f9d522eaa28672ada8d271a5eeb0bd139721307961024730784389efe99b54c43aa07e1a7d17de1ac8e0a51fb8ba4b4fa640df9600a170768c541921c9513607f2ac5787f6de99355dd17647844de38ee6f2428142f1a55a0247d7d38a1c3d2d7f7046b8c361fd7e08427cbb37ce03b1774c6dfdc01ebca9344799a833e216a8f312038037db42634bef17e9fee72697e3c7d946522c20c8f9b657d6167ef7ea77d72688c77a62b4e05749144c146bfe3faf4bd4a0ea5cdda16b0545711e29db13dfed4baaecc4a26edfffa3758eec57747e9e5541f0a11830689c888c5a8787c9807c55e7493675ae20815ea9efc8ca21d627bc013c506c7c25c54ddf6c4dc2fc2b52850179d593f517539eb1a95f14497089e3613cd9ac8f0c43d3e610ba4b67c3d658201a1e5d0da4d748647fca9e0b310530610076e5725981d106200d8c4e6a05027a4d6157ff947589a68c1a7e435d49d9c2c14d1be9c3787a6c46d3393b92fd1401bec7a5c6c3c0da0d25a319f97fc671626576a7439b5916ea0d14370c37b481df32057875b4c56a59df61131619dd66609f403743566bad17342c38e71d52c69544e368facaecce210b0592584c81c9a19ba4dde51e6da7a0ee398f0d201f41b08a8c3cfe8e5b52e785368b91b992889048a8e199f6ce0bd32b12111e11af523d197756ad45f2edeca43bc01057e1aef0561b5808fdc9c24c47ef256d95dbfd0af8d8fb6143c9a8140290d6eed949956127576e12d0322634ab4e9c783799f9a22f4ac5d88b5d042d1dbb790130c62f0056eac565308169a559d5932b2535dc4d0c379998ecd090d8adbf2c9e065ad57128953288f368e234f97eb004eb13ff1b8fdfc0295a249f2eb0ad7a35cd6839b07ce21eef76ca5ae1674c9a9d78c6d0d5b5315e87e2c83e49ab0f98486e835f628a67e3d9a34d3990fedb5e6b5fcdcf0b8fd8d09b878e7b3873fa6084c9d5ae92b9e1bb93a2b353567821f36a0d934472ae5d2af8d10cea285a55e806d99317b01fcf5fc5ee54a696ff5db3fdad83f1842d4ebbbedfe0fcae66800cd32170a19024db3447f7251f9deb7968247be3573bd0c3a0f42c1791a5f37315c0488a63d10ef76331854bd0afa388002f374a538e7092cc6f6dd987ed8d7f63eae59b52b2663c28e9124fa90b3c301f267f3729a5f41a2dc075cbd744a3357aa85b0be44075eb81767f7a0da798b69ac2bcca958ff792384b22845e4d958f2ba8c9180d4");

        uint256[] memory public_inputs=new uint256[](6);

        uint256 blockId = 18446744069414584595 - P;
        assertEq( blockId, BLOCK_ID);


        public_inputs[0]=3544317610574872;
        public_inputs[1]=3818006324670434;
        public_inputs[2]=1609100126983798;
        public_inputs[3]=2043470627881931;
        public_inputs[4]=1883474428618504;
        public_inputs[5]=18446744069414584595;

        assertEq(prover.verifyProof(proof, public_inputs), true);

        bytes32 merkle_root = getMerkleRootFromPublicInput(public_inputs);
        console.logBytes32(merkle_root);

        uint256[] memory inputs = getPublicInputsFromMerkleRoot(merkle_root);
        
        /*NOT READY
        for(uint256 i = 0 ; i < 5 ; i ++ ){
            console.log(inputs[i], public_inputs[i]);
            assertEq(inputs[i], public_inputs[i] );
        }
        */
        assertEq(merkle_root, getMerkleRootFromPublicInput(inputs));
        console.logBytes32(merkle_root);



    }
}


