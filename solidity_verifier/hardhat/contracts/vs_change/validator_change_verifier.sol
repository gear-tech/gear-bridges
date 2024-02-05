pragma solidity >=0.7.0 <0.9.0;
import {Groth16Verifier} from "./final_verifier.sol";

contract ValidatorSetChangeVerifier is Groth16Verifier{
     uint[8] public  circuitDigestAndMerkleRoots;
     uint[5][] public validatorSet;
     uint public validatorSetId;

    event SuccessfulVerification(uint[5]);
    event VerificationFailed(uint[5]);

    constructor (uint[8] memory _circuitDigestAndMerkleRoots, uint[5] memory _validatorSet, uint  _validatorSetId) {
        circuitDigestAndMerkleRoots = _circuitDigestAndMerkleRoots;
        validatorSet.push(_validatorSet); 
        validatorSetId = _validatorSetId; 
    }

    function  verifyValidatorSetChangeProof(uint[2] calldata _pA, uint[2][2] calldata _pB, uint[2] calldata _pC, uint[5] calldata nextValidatorSet, uint _validatorSetId) public {
        require(validatorSetId == _validatorSetId, "Wrong validator set ID");
        validatorSetId = validatorSetId + 1;
        uint[19] memory publicInputs = getPublicInputs(nextValidatorSet, _validatorSetId);
        validatorSet.push(nextValidatorSet);
        bytes memory executePayload = abi.encodeWithSignature("verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[19])", _pA, _pB, _pC, publicInputs);
       (bool success, bytes memory returnData) = address(address(this)).call(executePayload);
        bool successful_verification = abi.decode(returnData, (bool));
        require(success && successful_verification, "Verification failed");
        emit SuccessfulVerification(nextValidatorSet);
    }

    function getPublicInputs(uint[5] calldata nextValidatorSet, uint _nonceId ) internal  view returns (uint[19] memory){
        uint index = validatorSet.length - 1;
        uint[5] storage prevValidatorSet = validatorSet[index];
        uint[19] memory publicInputs;

        for (uint i=0; i < circuitDigestAndMerkleRoots.length; i++) {
            publicInputs[i] = circuitDigestAndMerkleRoots[i];
        }

        for (uint i=0; i < prevValidatorSet.length; i++) {
            publicInputs[8 + i] = prevValidatorSet[i];
        }
         for (uint i=0; i < nextValidatorSet.length; i++) {
            publicInputs[13 + i]  = nextValidatorSet[i];
        }
        publicInputs[18] = _nonceId;
        return publicInputs;
    } 

    function getValidatorSetId() public view returns (uint) {
        return validatorSetId;
    }

    function getAllValidatorSets() public view returns (uint[5][] memory) {
        return validatorSet;
    }

    function getLastValidatorSet() public view returns (uint[5] memory) {
         uint index = validatorSet.length - 1;
        return validatorSet[index];
    }
}