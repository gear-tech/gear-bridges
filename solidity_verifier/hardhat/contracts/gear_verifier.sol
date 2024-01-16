pragma solidity >=0.7.0 <0.9.0;
import {Groth16Verifier} from "./final_verifier.sol";

contract ValidatorSetChangeVerifier is Groth16Verifier{
     uint[78] public  publicInputs;
     uint[5][] public validatorSet;
     uint public nonceId;
     bool public verified;
     
    event SuccessfulVerification(uint[5]);
    event VerificationFailed(uint[5]);

    constructor (uint[68] memory _circuitDigestAndMerkleRoots, uint[5] memory _validatorSet) {
        publicInputs = _circuitDigestAndMerkleRoots;
        validatorSet.push(_validatorSet); 
        nonceId = 0; 
        verified = false;
    }

    function  verifyValidatorSetChangeProof(uint[2] calldata _pA, uint[2][2] calldata _pB, uint[2] calldata _pC, uint[5] calldata nextValidatorSet) public {
        constructPublicInputs(nextValidatorSet);
        bytes memory executePayload = abi.encodeWithSignature("verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[78])", _pA, _pB, _pC, publicInputs);
       (bool success, bytes memory returnData) = address(address(this)).call(executePayload);
        bool successful_verification = abi.decode(returnData, (bool));
        if (success && successful_verification) {
            verified = true;
            validatorSet.push(nextValidatorSet);
            emit SuccessfulVerification(nextValidatorSet);
        } else {
            verified = false;
            emit VerificationFailed(nextValidatorSet);
        }
    }

    function constructPublicInputs(uint[5] calldata nextValidatorSet ) internal {
        uint index = validatorSet.length - 1;
        uint[5] storage prevValidatorSet = validatorSet[index];
        for (uint i=0; i < prevValidatorSet.length; i++) {
            publicInputs[68 + i] = prevValidatorSet[i];
        }
         for (uint i=0; i < nextValidatorSet.length; i++) {
            publicInputs[73 + i]  = nextValidatorSet[i];
        }
    }

    function getValidatorSet() public view returns (uint[5][] memory)  {
        return validatorSet;
    }

    function getVerified() public view returns (bool) {
        return verified;
    }
}