pragma solidity >=0.7.0 <0.9.0;
import {Groth16Verifier} from "./final_verifier.sol";

contract MessageSentVerifier is Groth16Verifier{
     uint[19] public  publicInputs;
     uint[5][] public validatorSet;
     uint public nonceId;

    event SuccessfulVerification(uint[5]);
    event VerificationFailed(uint[5]);

    constructor (uint[8] memory _circuitDigestAndMerkleRoots, uint[5] memory _validatorSet, uint  _nonceId) {
        publicInputs = _circuitDigestAndMerkleRoots;
        validatorSet.push(_validatorSet); 
        nonceId = _nonceId; 
    }

    function  verifyMsgSentProof(uint[2] calldata _pA, uint[2][2] calldata _pB, uint[2] calldata _pC, uint[5] calldata nextValidatorSet, uint _nonceId) public {
        nonceId = nonceId + 1;
        require(nonceId == _nonceId, "Wrong validator set ID");
        constructPublicInputs(nextValidatorSet);
        validatorSet.push(nextValidatorSet);
        bytes memory executePayload = abi.encodeWithSignature("verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[19])", _pA, _pB, _pC, publicInputs);
       (bool success, bytes memory returnData) = address(address(this)).call(executePayload);
        bool successful_verification = abi.decode(returnData, (bool));
        if (success && successful_verification) {
            emit SuccessfulVerification(nextValidatorSet);
        } else {
            emit VerificationFailed(nextValidatorSet);
        }
    }

    function constructPublicInputs(uint[5] calldata nextValidatorSet ) internal {
         uint index = validatorSet.length - 1;
        uint[5] storage prevValidatorSet = validatorSet[index];
        for (uint i=0; i < prevValidatorSet.length; i++) {
            publicInputs[8 + i] = prevValidatorSet[i];
        }
        publicInputs[13] = nonceId;
         for (uint i=0; i < nextValidatorSet.length; i++) {
            publicInputs[14 + i]  = nextValidatorSet[i];
        }
        
    } 

    function getPublicInputs() public view returns (uint[19] memory) {
        return publicInputs;
    }

    function getNonceId() public view returns (uint) {
        return nonceId;
    }

     function getAllValidatorSets() public view returns (uint[5][] memory) {
        return validatorSet;
    }

    function getLastValidatorSet() public view returns (uint[5] memory) {
         uint index = validatorSet.length - 1;
        return validatorSet[index];
    }
}