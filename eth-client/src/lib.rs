mod error;
use error::VerifierError;
use ethcontract::prelude::*;
use primitive_types::U256;
use serde::Deserialize;
use std::{env, fs};
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
        path_to_final_proof: &str,
        path_to_final_public: &str,
    ) -> Result<bool, VerifierError> {
        let (p_a, p_b, p_c) = get_coefficients(path_to_final_proof)?;
        let publics = get_publics(path_to_final_public)?;

        let validator_set = [
            publics[14],
            publics[15],
            publics[16],
            publics[17],
            publics[18],
        ];

        let nonce_id = publics[13];

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
        path_to_final_proof: &str,
        path_to_final_public: &str,
    ) -> Result<bool, VerifierError> {
        let (p_a, p_b, p_c) = get_coefficients(path_to_final_proof)?;
        let publics = get_publics(path_to_final_public)?;

        let account = {
            let pk = env::var("PK").expect("PK is not set");
            let key: PrivateKey = pk.parse().expect("invalid PK");
            Account::Offline(key, None)
        };

        let validator_set = [
            publics[14],
            publics[15],
            publics[16],
            publics[17],
            publics[18],
        ];

        let nonce_id = publics[13];

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

    pub async fn get_all_validator_sets_from_vs_vrf(
        &self,
    ) -> Result<Vec<[U256; 5]>, VerifierError> {

        Ok(self.vs_change_vrf
            .get_all_validator_sets()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?)
    }

    pub async fn get_last_validator_set_from_vs_vrf(
        &self,
    ) -> Result<[U256; 5], VerifierError> {

        Ok(self.vs_change_vrf
            .get_last_validator_set()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?)
    }

    pub async fn get_nonce_id_from_vs_vrf(
        &self,
    ) -> Result<U256, VerifierError> {

        Ok(self.vs_change_vrf
            .get_nonce_id()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?)
    }


    pub async fn get_all_validator_sets_from_msg_sent_vrf(
        &self,
    ) -> Result<Vec<[U256; 5]>, VerifierError> {

        Ok(self.msg_sent_vrf
            .get_all_validator_sets()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?)
    }

    pub async fn get_last_validator_set_from_msg_sent_vrf(
        &self,
    ) -> Result<[U256; 5], VerifierError> {

        Ok(self.msg_sent_vrf
            .get_last_validator_set()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?)
    }

    pub async fn get_nonce_id_from_msg_sent_vrf(
        &self,
    ) -> Result<U256, VerifierError> {

        Ok(self.msg_sent_vrf
            .get_nonce_id()
            .call()
            .await
            .map_err(|_| VerifierError::ErrorDuringContractExecution)?)
    }
}

fn get_coefficients(
    path_to_final_proof: &str,
) -> Result<([U256; 2], [[U256; 2]; 2], [U256; 2]), VerifierError> {
    let final_proof =
        fs::read_to_string(path_to_final_proof).map_err(|_| VerifierError::WrongPathToFile)?;

    #[derive(Debug, Deserialize)]
    struct FinalProof {
        pi_a: Vec<String>,
        pi_b: Vec<Vec<String>>,
        pi_c: Vec<String>,
        protocol: String,
        curve: String,
    }
    let final_proof: FinalProof =
        serde_json::from_str(&final_proof).map_err(|_| VerifierError::WrongJsonFormation)?;

    let p_a = [
        U256::from_dec_str(&final_proof.pi_a[0])
            .map_err(|_| VerifierError::UnableToConvertToU256)?,
        U256::from_dec_str(&final_proof.pi_a[1])
            .map_err(|_| VerifierError::UnableToConvertToU256)?,
    ];
    let p_b = [
        [
            U256::from_dec_str(&final_proof.pi_b[0][1])
                .map_err(|_| VerifierError::UnableToConvertToU256)?,
            U256::from_dec_str(&final_proof.pi_b[0][0])
                .map_err(|_| VerifierError::UnableToConvertToU256)?,
        ],
        [
            U256::from_dec_str(&final_proof.pi_b[1][1])
                .map_err(|_| VerifierError::UnableToConvertToU256)?,
            U256::from_dec_str(&final_proof.pi_b[1][0])
                .map_err(|_| VerifierError::UnableToConvertToU256)?,
        ],
    ];
    let p_c = [
        U256::from_dec_str(&final_proof.pi_c[0])
            .map_err(|_| VerifierError::UnableToConvertToU256)?,
        U256::from_dec_str(&final_proof.pi_c[1])
            .map_err(|_| VerifierError::UnableToConvertToU256)?,
    ];
    Ok((p_a, p_b, p_c))
}

fn get_publics(path_to_final_public: &str) -> Result<[U256; 19], VerifierError> {
    let final_public =
        fs::read_to_string(path_to_final_public).map_err(|_| VerifierError::WrongPathToFile)?;

    let final_public: Vec<String> =
        serde_json::from_str(&final_public).map_err(|_| VerifierError::WrongJsonFormation)?;

    let publics: Vec<U256> = final_public
        .into_iter()
        .map(|x| U256::from_dec_str(&x).unwrap())
        .collect();

    let publics: [U256; 19] = publics
        .try_into()
        .map_err(|_| VerifierError::UnableToConvertToU256)?;

    Ok(publics)
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[tokio::test]
    async fn validator_set_change_verifier() {
        let vs_change_vrf_address = "5FbDB2315678afecb367f032d93F642f64180aa3";
        let msg_sent_vrf_address = "e7f1725E7734CE288F8367e1Bb143E90bb3F0512";
        let url = "http://127.0.0.1:8545/";
        let verifier = ContractVerifiers::new(url, vs_change_vrf_address, msg_sent_vrf_address)
            .expect("Error during verifiers instantiation");

        println!(
            "{:?}",
            verifier
                .verify_vs_change(
                    "../solidity_verifier/aggregation/vs_change/final_proof.json",
                    "../solidity_verifier/aggregation/vs_change/final_public.json"
                )
                .await
        );

        println!(
            "{:?}",
            verifier
                .get_all_validator_sets_from_vs_vrf()
                .await
        );

        println!(
            "{:?}",
            verifier
                .get_last_validator_set_from_vs_vrf()
                .await
        );

        println!(
            "{:?}",
            verifier
                .get_nonce_id_from_vs_vrf()
                .await
        );
    }

    #[tokio::test]
    async fn msg_sent_verifier() {
        let vs_change_vrf_address = "5FbDB2315678afecb367f032d93F642f64180aa3";
        let msg_sent_vrf_address = "e7f1725E7734CE288F8367e1Bb143E90bb3F0512";
        let url = "http://127.0.0.1:8545/";
        let verifier = ContractVerifiers::new(url, vs_change_vrf_address, msg_sent_vrf_address)
            .expect("Error during verifiers instantiation");

        println!(
            "{:?}",
            verifier
                .verify_msg_sent(
                    "../solidity_verifier/aggregation/message_sent/final_proof.json",
                    "../solidity_verifier/aggregation/message_sent/final_public.json"
                )
                .await
        );

        println!(
            "{:?}",
            verifier
                .get_all_validator_sets_from_msg_sent_vrf()
                .await
        );

        println!(
            "{:?}",
            verifier
                .get_last_validator_set_from_msg_sent_vrf()
                .await
        );

        println!(
            "{:?}",
            verifier
                .get_nonce_id_from_msg_sent_vrf()
                .await
        );
    }
}
