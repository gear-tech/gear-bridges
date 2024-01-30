pragma solidity >=0.7.0 <0.9.0;
import {Groth16Verifier} from "./final_verifier.sol";
import {ValidatorSetChangeVerifier} from "../vs_change/validator_change_verifier.sol";

contract MessageSentVerifier is Groth16Verifier{
     uint[8] public  circuitDigestAndMerkleRoots;
     uint[5][] public msgHashes;
    ValidatorSetChangeVerifier public vsChangeVerifier;

    event SuccessfulVerification(uint[5]);

    constructor (uint[8] memory _circuitDigestAndMerkleRoots, address  _vsChangeVerifierAddr) {
        circuitDigestAndMerkleRoots = _circuitDigestAndMerkleRoots;
        vsChangeVerifier = ValidatorSetChangeVerifier(_vsChangeVerifierAddr); 
    }

    function  verifyMsgSentProof(uint[2] calldata _pA, uint[2][2] calldata _pB, uint[2] calldata _pC, uint[5] calldata _msgHashes, uint _validatorSetId) public {
        uint validatorSetId = vsChangeVerifier.getValidatorSetId();
        require(validatorSetId == _validatorSetId, "Wrong validator set ID");
        uint[5] memory validatorSet = vsChangeVerifier.getLastValidatorSet();
        uint[19] memory publicInputs = getPublicInputs(validatorSet, _msgHashes, validatorSetId);
        msgHashes.push(_msgHashes);
        bytes memory executePayload = abi.encodeWithSignature("verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[19])", _pA, _pB, _pC, publicInputs);
       (bool success, bytes memory returnData) = address(address(this)).call(executePayload);
        bool successful_verification = abi.decode(returnData, (bool));
        require(success && successful_verification, "Verification failed");
        emit SuccessfulVerification(_msgHashes);   
    }

    function getPublicInputs(uint[5] memory validatorSet, uint[5] calldata _msgHashes, uint validatorSetId) internal  view returns (uint[19] memory) {
        uint[19] memory publicInputs;

        for (uint i=0; i < circuitDigestAndMerkleRoots.length; i++) {
            publicInputs[i] = circuitDigestAndMerkleRoots[i];
        }

        for (uint i=0; i < validatorSet.length; i++) {
            publicInputs[8 + i] = validatorSet[i];
        }
        publicInputs[13] = validatorSetId;
         for (uint i=0; i < _msgHashes.length; i++) {
            publicInputs[14 + i]  = _msgHashes[i];
        }
        return publicInputs;
    } 

    function getAllMsgHashes() public view returns (uint[5][] memory) {
        return msgHashes;
    }

    function getLastMsgHashes() public view returns (uint[5] memory) {
         uint index = msgHashes.length - 1;
        return msgHashes[index];
    }
}