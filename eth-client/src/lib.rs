mod error;
use error::VerifierError;
use ethcontract::prelude::*;
use primitive_types::U256;
use std::env;
ethcontract::contract!("solidity_verifier/hardhat/artifacts/contracts/gear_verifier.sol/ValidatorSetChangeVerifier.json");

pub struct ContractVerifier(validator_set_change_verifier::Contract);

impl ContractVerifier {
    pub fn new(url: &str, address: &str) -> Result<Self, VerifierError> {
        let http = Http::new(url).map_err(|_| VerifierError::ErrorInHTTPTransport)?;
        let web3 = Web3::new(http);
        let address = address
            .parse::<web3::types::Address>()
            .map_err(|_| VerifierError::WrongAddress)?;
        Ok(ContractVerifier(ValidatorSetChangeVerifier::at(
            &web3, address,
        )))
    }

    pub async fn verify(
        &self,
        p_a: [U256; 2],
        p_b: [[U256; 2]; 2],
        p_c: [U256; 2],
        validator_set: [U256; 5],
    ) -> Result<bool, VerifierError> {
        let account = {
            let pk = env::var("PK").expect("PK is not set");
            let key: PrivateKey = pk.parse().expect("invalid PK");
            Account::Offline(key, None)
        };
        println!(
            "{:?}",
            account
        );

        println!("validator set {:?}", validator_set);
        self.0
            .verify_validator_set_change_proof(p_a, p_b, p_c, validator_set)
            .from(account)
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?;

        let verified = self
            .0
            .get_verified()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?;

        let set = self
            .0
            .get_validator_set()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?;
        println!("{:?}", set);
        Ok(verified)
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use serde::Deserialize;
    use std::fs;
    #[tokio::test]
    async fn final_verifier() {
        let address = "8A791620dd6260079BF849Dc5567aDC3F2FdC318";
        let url = "http://127.0.0.1:8545/";
        let verifier = ContractVerifier::new(url, address).expect("");

        let final_proof = fs::read_to_string("../solidity_verifier/aggregation/final_proof.json")
            .expect("Unable to open the file");
        let final_public = fs::read_to_string("../solidity_verifier/aggregation/final_public.json")
            .expect("Unable to open the file");

        #[derive(Debug, Deserialize)]
        struct FinalProof {
            pi_a: Vec<String>,
            pi_b: Vec<Vec<String>>,
            pi_c: Vec<String>,
            protocol: String,
            curve: String,
        }

        let final_proof: FinalProof =
            serde_json::from_str(&final_proof).expect("JSON was not well-formatted");
        let final_public: Vec<String> =
            serde_json::from_str(&final_public).expect("JSON was not well-formatted");

        let p_a = [
            U256::from_dec_str(&final_proof.pi_a[0]).unwrap(),
            U256::from_dec_str(&final_proof.pi_a[1]).unwrap(),
        ];
        let p_b = [
            [
                U256::from_dec_str(&final_proof.pi_b[0][1]).unwrap(),
                U256::from_dec_str(&final_proof.pi_b[0][0]).unwrap(),
            ],
            [
                U256::from_dec_str(&final_proof.pi_b[1][1]).unwrap(),
                U256::from_dec_str(&final_proof.pi_b[1][0]).unwrap(),
            ],
        ];
        let p_c = [
            U256::from_dec_str(&final_proof.pi_c[0]).unwrap(),
            U256::from_dec_str(&final_proof.pi_c[1]).unwrap(),
        ];
        let publics: Vec<U256> = final_public
            .into_iter()
            .map(|x| U256::from_dec_str(&x).unwrap())
            .collect();
        let publics: [U256; 78] = publics.try_into().unwrap();
        let (_, validator_set) = publics.split_at(73);

        println!(
            "{:?}",
            verifier
                .verify(p_a, p_b, p_c, validator_set.try_into().unwrap())
                .await
        );
    }
}
