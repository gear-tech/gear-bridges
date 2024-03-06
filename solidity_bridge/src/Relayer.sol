pragma solidity ^0.8.24;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";
import {IProver} from "./interfaces/IProver.sol";
import {IRelayer} from "./interfaces/IRelayer.sol";
import {Constants} from "./libraries/Constants.sol";



contract Relayer is IRelayer, AccessControl {
    IProver private _prover;
    mapping(uint256=>bytes32) private _block_numbers;
    mapping(bytes32=>uint256) private _merkle_roots;

    uint256 private constant P = 2**64 - 2**32 + 1;
    uint256 private constant MASK_52BITS = (2**52) - 1;


    function initialize(address prover) external {
        if(getRoleAdmin(Constants.ADMIN_ROLE) != DEFAULT_ADMIN_ROLE) revert AlreadyInitialized();
        _setRoleAdmin(Constants.ADMIN_ROLE, Constants.ADMIN_ROLE);
        _grantRole(Constants.ADMIN_ROLE, msg.sender );
        _prover = IProver(prover);
    }

    function add_merkle_root_with_block(uint256 block_number, bytes32 merkle_root, bytes calldata proof ) external {
        uint256[] memory public_inputs = new uint256[](6); 

        uint256 _merkle_root=uint256(merkle_root);
        for(uint256 i = 0; i < 5; i++ ){
            public_inputs[i] = (_merkle_root & MASK_52BITS);
            _merkle_root >>= 52;
        }
        public_inputs[5] = block_number + P;

        if(!_prover.verifyProof(proof, public_inputs)) {
            revert InvalidProof();
        }
        _block_numbers[block_number] = merkle_root;
        _merkle_roots[merkle_root] = block_number;
    }
    
    function add_merkle_root_with_inputs(uint256[] calldata public_inputs, bytes calldata proof ) external {
        if(!_prover.verifyProof(proof, public_inputs)) {
            revert InvalidProof();
        }

        uint256 merkle_root=uint256(public_inputs[4] & MASK_52BITS) % P;
        for(uint256 i = 4 ; i > 0; i --) {
            merkle_root <<= 52;
            merkle_root |= (public_inputs[i-1] & MASK_52BITS) % P;
        }

        uint256 block_number = public_inputs[5] - P;

        _block_numbers[block_number] = bytes32(merkle_root);
        _merkle_roots[bytes32(merkle_root)] = block_number;
    }

    function get_merkle_root(uint256 block_number) external view returns(bytes32) {
        return _block_numbers[block_number];
    }

    function get_block_number(bytes32 merkle_root) external view returns(uint256) {
        return _merkle_roots[merkle_root];
    }



}