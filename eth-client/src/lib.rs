mod error;
use error::VerifierError;
use ethcontract::prelude::*;
use primitive_types::U256;
use std::env;
ethcontract::contract!("solidity_verifier/hardhat/artifacts/contracts/vs_change/validator_change_verifier.sol/ValidatorSetChangeVerifier.json");
ethcontract::contract!("solidity_verifier/hardhat/artifacts/contracts/message_sent/message_sent_verifier.sol/MessageSentVerifier.json");

pub struct ContractVerifiers {
    vs_change_vrf: validator_set_change_verifier::Contract,
    msg_sent_vrf: message_sent_verifier::Contract,
}

impl ContractVerifiers {
    pub fn new(
        url: &str,
        address_vs_change: &str,
        address_msg_sent: &str,
    ) -> Result<Self, VerifierError> {
        let http = Http::new(url).map_err(|_| VerifierError::ErrorInHTTPTransport)?;
        let web3 = Web3::new(http);
        let address_vs_change = address_vs_change
            .parse::<web3::types::Address>()
            .map_err(|_| VerifierError::WrongAddress)?;

        let address_msg_sent = address_msg_sent
            .parse::<web3::types::Address>()
            .map_err(|_| VerifierError::WrongAddress)?;

        Ok(ContractVerifiers {
            vs_change_vrf: ValidatorSetChangeVerifier::at(&web3, address_vs_change),
            msg_sent_vrf: MessageSentVerifier::at(&web3, address_msg_sent),
        })
    }

    pub async fn verify_vs_change(
        &self,
        p_a: [U256; 2],
        p_b: [[U256; 2]; 2],
        p_c: [U256; 2],
        validator_set: [U256; 5],
        nonce_id: U256,
    ) -> Result<bool, VerifierError> {
        let account = {
            let pk = env::var("PK").expect("PK is not set");
            let key: PrivateKey = pk.parse().expect("invalid PK");
            Account::Offline(key, None)
        };

        self.vs_change_vrf
            .verify_validator_set_change_proof(p_a, p_b, p_c, validator_set, nonce_id)
            .from(account)
            .send()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?;

        let exp_nonce_id = self
            .vs_change_vrf
            .get_nonce_id()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?;
        if exp_nonce_id != nonce_id {
            return Ok(false);
        }
        Ok(true)
    }

    pub async fn verify_msg_sent(
        &self,
        p_a: [U256; 2],
        p_b: [[U256; 2]; 2],
        p_c: [U256; 2],
        validator_set: [U256; 5],
        nonce_id: U256,
    ) -> Result<bool, VerifierError> {
        let account = {
            let pk = env::var("PK").expect("PK is not set");
            let key: PrivateKey = pk.parse().expect("invalid PK");
            Account::Offline(key, None)
        };

        self.msg_sent_vrf
            .verify_msg_sent_proof(p_a, p_b, p_c, validator_set, nonce_id)
            .from(account)
            .send()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?;

        let exp_nonce_id = self
            .msg_sent_vrf
            .get_nonce_id()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?;
        if exp_nonce_id != nonce_id {
            return Ok(false);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use serde::Deserialize;
    use std::fs;
    #[tokio::test]
    async fn validator_set_change_verifier() {
        let vs_change_vrf_address = "9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0";
        let msg_sent_vrf_address = "2279B7A0a67DB372996a5FaB50D91eAA73d2eBe6";
        let url = "http://127.0.0.1:8545/";
        let verifier = ContractVerifiers::new(url, vs_change_vrf_address, msg_sent_vrf_address)
            .expect("Error during verifiers instantiation");

        let final_proof =
            fs::read_to_string("../solidity_verifier/aggregation/vs_change/final_proof.json")
                .expect("Unable to open the file");
        let final_public =
            fs::read_to_string("../solidity_verifier/aggregation/vs_change/final_public.json")
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
        let publics: [U256; 19] = publics.try_into().unwrap();
        let validator_set = [
            publics[13],
            publics[14],
            publics[15],
            publics[16],
            publics[17],
        ];

        println!(
            "{:?}",
            verifier
                .verify_vs_change(p_a, p_b, p_c, validator_set, publics[18])
                .await
        );
    }

    #[tokio::test]
    async fn msg_sent_verifier() {
        let vs_change_vrf_address = "9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0";
        let msg_sent_vrf_address = "2279B7A0a67DB372996a5FaB50D91eAA73d2eBe6";
        let url = "http://127.0.0.1:8545/";
        let verifier = ContractVerifiers::new(url, vs_change_vrf_address, msg_sent_vrf_address)
            .expect("Error during verifiers instantiation");

        let final_proof =
            fs::read_to_string("../solidity_verifier/aggregation/message_sent/final_proof.json")
                .expect("Unable to open the file");
        let final_public =
            fs::read_to_string("../solidity_verifier/aggregation/message_sent/final_public.json")
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
        let publics: [U256; 19] = publics.try_into().unwrap();
        let validator_set = [
            publics[14],
            publics[15],
            publics[16],
            publics[17],
            publics[18],
        ];

        println!(
            "{:?}",
            verifier
                .verify_msg_sent(p_a, p_b, p_c, validator_set, publics[13])
                .await
        );
    }
}
