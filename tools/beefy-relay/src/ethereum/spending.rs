use super::*;
use alloy::rpc::types::TransactionRequest;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap, fs::File, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf,
};

// Pending originals remain charged across UTC days. Finalized originals are charged
// their admitted worst case, not an unauthenticated RPC gas-used estimate.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    version: u8,
    chain_id: u64,
    genesis: B256,
    sender: Address,
    day: u64,
    settled: U256,
    reservations: BTreeMap<B256, Reservation>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    role: String,
    approval: B256,
    request: TransactionRequest,
    maximum_cost: U256,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    reservation: Reservation,
    transaction_hash: B256,
    block_hash: B256,
}

fn request_identity(request: &TransactionRequest) -> Result<B256> {
    let mut request = request.clone();
    request.gas = None;
    request.max_fee_per_gas = None;
    request.max_priority_fee_per_gas = None;
    Ok(B256::from(keccak256(&serde_json::to_vec(&request)?)))
}

fn validate_envelope(
    reservation: &Reservation,
    raw: &[u8],
    sender: Address,
    chain: u64,
) -> Result<()> {
    let request = &reservation.request;
    let destination = request
        .to
        .context("HOLD: admitted destination missing")?
        .to()
        .copied()
        .context("HOLD: own signing cannot create contracts")?;
    let (signed, identity) = signed_transaction(
        raw,
        B256::from(keccak256(raw)),
        request.nonce.context("HOLD: admitted nonce missing")?,
        destination,
    )?;
    ensure!(
        matches!(signed, TxEnvelope::Eip1559(_))
            && signed.chain_id() == Some(chain)
            && identity.from == sender
            && Some(signed.gas_limit()) == request.gas
            && Some(signed.max_fee_per_gas()) == request.max_fee_per_gas
            && signed.max_priority_fee_per_gas() == request.max_priority_fee_per_gas
            && signed.value() == request.value.unwrap_or(U256::ZERO)
            && signed.input().as_ref()
                == request
                    .input
                    .input()
                    .map(|input| input.as_ref())
                    .unwrap_or(&[]),
        "HOLD: signed envelope differs from durable spend admission"
    );
    ensure!(
        signed.access_list().is_none_or(|list| list.is_empty()),
        "HOLD: unexpected signed access list"
    );
    Ok(())
}

impl Ledger {
    fn load(path: &Path, chain_id: u64, genesis: B256, sender: Address) -> Result<Self> {
        let temporary = PathBuf::from(format!("{}.tmp", path.display()));
        if gear_common::durable::entry_exists(&temporary)? {
            let committed = if gear_common::durable::entry_exists(path)? {
                ensure!(
                    fs::symlink_metadata(path)?.file_type().is_file(),
                    "HOLD: nonregular spend ledger"
                );
                Some(serde_json::from_slice(&fs::read(path)?)?)
            } else {
                None
            };
            crate::follower::preserve_owned_scratch(&temporary, committed.as_ref())?;
        }
        let ledger: Self = if gear_common::durable::entry_exists(path)? {
            serde_json::from_slice(&gear_common::durable::committed_bytes(path)?)?
        } else {
            Self {
                version: 1,
                chain_id,
                genesis,
                sender,
                day: 0,
                settled: U256::ZERO,
                reservations: BTreeMap::new(),
            }
        };
        ensure!(
            ledger.version == 1
                && ledger.chain_id == chain_id
                && ledger.genesis == genesis
                && ledger.sender == sender
                && ledger.reservations.len() <= 128,
            "HOLD: spend ledger identity or capacity changed"
        );
        Ok(ledger)
    }

    fn advance_day(&mut self, day: u64) -> Result<()> {
        ensure!(
            day >= self.day,
            "HOLD: finalized spending clock moved backwards"
        );
        if day > self.day {
            self.day = day;
            self.settled = U256::ZERO;
        }
        Ok(())
    }

    fn committed_cost(&self) -> Result<U256> {
        self.reservations
            .values()
            .try_fold(self.settled, |sum, reservation| {
                sum.checked_add(reservation.maximum_cost)
                    .context("HOLD: spend reservation overflow")
            })
    }

    fn save(&self, path: &Path) -> Result<()> {
        let temporary = PathBuf::from(format!("{}.tmp", path.display()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        serde_json::to_writer(&mut file, self)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, path)?;
        File::open(path.parent().context("spend ledger parent missing")?)?.sync_all()?;
        Ok(())
    }
}

impl Ethereum {
    pub(crate) fn ensure_signer_owner(&self) -> Result<()> {
        let (chain, genesis) = self.api.finality_identity()?;
        let identity = (chain, genesis, self.api.sender_address().0);
        if let Some((owned, _)) = self.signer_owner.get() {
            ensure!(
                *owned == identity,
                "HOLD: connected signer differs from retained host ownership"
            );
        } else {
            let owner = gear_common::signer_ownership::lock_ethereum_signer(
                chain,
                genesis.0.into(),
                identity.2,
            )?;
            self.signer_owner
                .set((identity, owner))
                .map_err(|_| anyhow::anyhow!("HOLD: concurrent signer owner acquisition"))?;
        }
        Ok(())
    }

    pub(crate) fn execution_signer_role(&self, role: &str) -> Result<()> {
        let sender = Address::from(self.api.sender_address().0);
        let approval = self
            .execution
            .as_ref()
            .context("HOLD: fresh signing lacks execution approval")?;
        ensure!(
            approval.signer_role(sender)? == role,
            "HOLD: account belongs to another execution role"
        );
        self.ensure_signer_owner()
    }

    fn spend_path(&self) -> Result<PathBuf> {
        self.ensure_signer_owner()?;
        let (chain, genesis) = self.api.finality_identity()?;
        gear_common::signer_ownership::ethereum_spend_ledger_path(
            chain,
            genesis.0.into(),
            self.api.sender_address().0,
        )
    }

    fn spending_policy(&self, role: &str) -> Result<(B256, crate::execution::SigningPolicy)> {
        let sender = Address::from(self.api.sender_address().0);
        if let Some(approval) = &self.execution {
            return Ok((approval.digest, approval.signing_policy(role, sender)?));
        }
        #[cfg(test)]
        if self._anvil.is_some()
            && self.verifier_address == Address::ZERO
            && self.receiver_address == Address::ZERO
            && self.api.finality_identity()?.0 == 31_337
            && role == "commitment"
        {
            let digest = B256::from(sp_core::hashing::sha2_256(&serde_json::to_vec(
                &crate::fixtures::generate_fixtures()?,
            )?));
            return Ok((
                digest,
                crate::execution::SigningPolicy {
                    commitment_gas_units: 30_000_000,
                    root_gas_units: 30_000_000,
                    max_fee_per_gas: 100_000_000_000,
                    max_priority_fee_per_gas: 100_000_000_000,
                    max_transaction_cost: U256::from(3_000_000_000_000_000_000u64),
                    daily_budget: U256::from(30_000_000_000_000_000_000u128),
                    wallet_warning_reserve: U256::from(1_000_000_000_000_000u64),
                },
            ));
        }
        anyhow::bail!("HOLD: fresh signing requires independently approved spending limits")
    }

    pub(crate) async fn admit_spend(
        &self,
        role: &str,
        request: &mut TransactionRequest,
        id: B256,
    ) -> Result<()> {
        let path = self.spend_path()?;
        let (approval, policy) = self.spending_policy(role)?;
        let gas_ceiling = policy.gas_limit(role)?;
        let (chain, genesis) = self.api.finality_identity()?;
        let sender = Address::from(self.api.sender_address().0);
        let destination = if role == "root" {
            self.queue_address
        } else {
            self.client_address
        };
        ensure!(
            request.to == Some(alloy::primitives::TxKind::Call(destination))
                && request.value.unwrap_or(U256::ZERO) == U256::ZERO
                && request.gas_price.is_none()
                && request
                    .access_list
                    .as_ref()
                    .is_none_or(|list| list.is_empty()),
            "HOLD: unexpected economic call in spend admission"
        );
        ensure!(
            request.from.is_none_or(|from| from == sender)
                && request.chain_id.is_none_or(|id| id == chain)
                && request.transaction_type.is_none_or(|kind| kind == 2),
            "HOLD: unsigned signing identity changed"
        );
        request.from = Some(sender);
        request.chain_id = Some(chain);
        request.transaction_type = Some(2);
        request.value = Some(U256::ZERO);
        let nonce = request
            .nonce
            .context("HOLD: spend admission requires a reserved nonce")?;
        let (latest, pending) = self.api.qualified_account_nonces(sender).await?;
        ensure!(
            latest == nonce && pending == nonce,
            "HOLD: spend admission nonce is unresolved"
        );
        let mut ledger = Ledger::load(&path, chain, genesis, sender)?;
        if let Some(saved) = ledger.reservations.get(&id) {
            ensure!(
                saved.approval == approval
                    && saved.role == role
                    && request_identity(&saved.request)? == request_identity(request)?,
                "HOLD: retained spend intent changed; no repricing or replacement"
            );
            *request = saved.request.clone();
            return Ok(());
        }
        ensure!(
            !gear_common::durable::entry_exists(&completion_path(&path, id))?,
            "HOLD: finalized spend reservation cannot be signed again"
        );
        // ponytail: 128 unresolved originals per account; reconcile them before admitting more.
        ensure!(
            ledger.reservations.len() < 128,
            "HOLD: unresolved spend capacity reached"
        );
        ensure!(
            ledger
                .reservations
                .values()
                .all(|saved| saved.request.nonce != Some(nonce)),
            "HOLD: another durable spend intent owns this nonce"
        );
        let view = self.api.verified_finalized_view().await?;
        let provider = self.api.raw_provider();
        let witness = self.api.raw_witness_provider()?;
        let (first, second): (alloy::primitives::U64, alloy::primitives::U64) = tokio::try_join!(
            provider.raw_request("eth_estimateGas".into(), (&*request, view.block_id())),
            witness.raw_request("eth_estimateGas".into(), (&*request, view.block_id()))
        )?;
        let gas = first.to::<u64>();
        ensure!(
            first == second && gas > 0 && gas <= gas_ceiling,
            "HOLD: paired gas quote disagrees or exceeds approved limit"
        );
        let (fees, second) = tokio::try_join!(
            provider.estimate_eip1559_fees(),
            witness.estimate_eip1559_fees()
        )?;
        ensure!(
            fees.max_fee_per_gas == second.max_fee_per_gas
                && fees.max_priority_fee_per_gas == second.max_priority_fee_per_gas
                && fees.max_fee_per_gas <= policy.max_fee_per_gas
                && fees.max_priority_fee_per_gas <= policy.max_priority_fee_per_gas
                && fees.max_priority_fee_per_gas <= fees.max_fee_per_gas,
            "HOLD: paired fee quote disagrees or exceeds approved limits"
        );
        let maximum_cost = U256::from(gas)
            .checked_mul(U256::from(fees.max_fee_per_gas))
            .context("HOLD: maximum transaction cost overflow")?;
        ensure!(
            maximum_cost > U256::ZERO && maximum_cost <= policy.max_transaction_cost,
            "HOLD: maximum transaction cost exceeds approval"
        );
        ledger.advance_day(view.checkpoint().common_header().timestamp / 86_400)?;
        let committed = ledger
            .committed_cost()?
            .checked_add(maximum_cost)
            .context("HOLD: daily spending overflow")?;
        ensure!(
            committed <= policy.daily_budget,
            "HOLD: daily spending budget exhausted"
        );
        let outstanding = committed
            .checked_sub(ledger.settled)
            .context("HOLD: invalid settled spend")?;
        let account = ethereum_client::inclusion::prove_account(
            provider,
            &view,
            view.checkpoint().common_header().clone(),
            sender,
        )
        .await?
        .verify(&view, sender)?
        .context("HOLD: signing account absent at finalized pin")?;
        let second_balance = witness
            .get_balance(sender)
            .block_id(view.block_id())
            .await?;
        ensure!(
            second_balance == account.balance && account.balance >= outstanding,
            "HOLD: corroborated finalized balance does not cover admitted spending"
        );
        if account.balance.saturating_sub(outstanding) < policy.wallet_warning_reserve {
            eprintln!("WARNING: {role} signer {sender} is below its approved wallet reserve; no automatic top-up");
        }
        view.recheck(provider).await?;
        request.gas = Some(gas);
        request.max_fee_per_gas = Some(fees.max_fee_per_gas);
        request.max_priority_fee_per_gas = Some(fees.max_priority_fee_per_gas);
        ledger.reservations.insert(
            id,
            Reservation {
                role: role.into(),
                approval,
                request: request.clone(),
                maximum_cost,
            },
        );
        ledger.save(&path)
    }

    pub(crate) async fn validate_admitted_spend(
        &self,
        role: &str,
        id: B256,
        raw: &[u8],
    ) -> Result<()> {
        let path = self.spend_path()?;
        let (chain, genesis) = self.api.finality_identity()?;
        let sender = Address::from(self.api.sender_address().0);
        let ledger = Ledger::load(&path, chain, genesis, sender)?;
        let saved = ledger
            .reservations
            .get(&id)
            .context("HOLD: signed original has no durable spend reservation")?;
        ensure!(
            saved.role == role,
            "HOLD: signed original changed spending role"
        );
        validate_envelope(saved, raw, sender, chain)
    }

    pub(crate) async fn finalize_commitment_spend(
        &self,
        original: &Value,
        finalized: &ethereum_client::FinalizedTransactionReceipt,
    ) -> Result<()> {
        if original["spendReservationId"].is_null() {
            return Ok(());
        }
        let id: B256 = serde_json::from_value(original["spendReservationId"].clone())?;
        ensure!(
            original["rawTransaction"] == bytes(&finalized.inclusion.raw_transaction),
            "HOLD: finalized spending evidence belongs to another commitment original"
        );
        self.finalize_spend(id, &finalized.inclusion, &finalized.checkpoint)
            .await
    }

    pub(crate) async fn finalize_spend(
        &self,
        id: B256,
        proof: &RecordedReceiptProof,
        checkpoint: &ethereum_client::FinalityCheckpoint,
    ) -> Result<()> {
        let path = self.spend_path()?;
        let (chain, genesis) = self.api.finality_identity()?;
        let sender = Address::from(self.api.sender_address().0);
        let hash = B256::from(keccak256(&proof.raw_transaction));
        let mut view = self.api.verified_finalized_view().await?;
        view.verify_checkpoint(self.api.raw_provider(), checkpoint)
            .await?;
        view.verify_blocks(
            self.api.raw_provider(),
            &BTreeMap::from([(proof.header.number, proof.header.hash_slow())]),
        )
        .await?;
        proof.verify_inclusion(&view, hash)?;
        view.recheck(self.api.raw_provider()).await?;
        let mut ledger = Ledger::load(&path, chain, genesis, sender)?;
        let completed = completion_path(&path, id);
        if gear_common::durable::entry_exists(&completed)? {
            let old: Completion =
                serde_json::from_slice(&gear_common::durable::committed_bytes(&completed)?)?;
            ensure!(
                old.transaction_hash == hash && old.block_hash == proof.header.hash_slow(),
                "HOLD: finalized spend inclusion changed"
            );
            validate_envelope(&old.reservation, &proof.raw_transaction, sender, chain)?;
            if !ledger.reservations.contains_key(&id) {
                return Ok(());
            }
        }
        let reservation = ledger
            .reservations
            .remove(&id)
            .context("HOLD: finalized original has no spend reservation")?;
        validate_envelope(&reservation, &proof.raw_transaction, sender, chain)?;
        ledger.advance_day(view.checkpoint().common_header().timestamp / 86_400)?;
        if proof.header.timestamp / 86_400 == ledger.day {
            ledger.settled = ledger
                .settled
                .checked_add(reservation.maximum_cost)
                .context("HOLD: settled spend overflow")?;
        }
        let directory = completed
            .parent()
            .context("spend completion directory missing")?;
        if !gear_common::durable::entry_exists(directory)? {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(directory)?;
        }
        ensure!(
            fs::symlink_metadata(directory)?.file_type().is_dir(),
            "HOLD: non-directory spend archive"
        );
        gear_common::durable::immutable_json(
            &completed,
            &Completion {
                reservation,
                transaction_hash: hash,
                block_hash: proof.header.hash_slow(),
            },
        )?;
        ledger.save(&path)
    }
}

fn completion_path(path: &Path, id: B256) -> PathBuf {
    path.with_extension("completed")
        .join(format!("{id:x}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn owned_original_spend_survives_retry_and_finalizes_once() -> Result<()> {
        let (mut ethereum, anchor, leaf, snapshot, proof) =
            super::super::tests::commitment_signing_fixture().await?;
        let mut original = Value::Null;
        assert!(ethereum
            .submit_synthetic_commitment(&anchor, &leaf, &snapshot, &proof, None, |record, _| {
                original = record.clone();
                ensure!(
                    record["rawTransaction"].is_null(),
                    "owned interruption after signing"
                );
                Ok(())
            })
            .await
            .is_err());
        let id: B256 = serde_json::from_value(original["spendReservationId"].clone())?;
        let path = ethereum.spend_path()?;
        let before = fs::read(&path)?;
        let ledger: Ledger = serde_json::from_slice(&before)?;
        let reservation = ledger
            .reservations
            .get(&id)
            .context("original reservation missing")?;
        let maximum_cost = reservation.maximum_cost;
        let mut request = reservation.request.clone();
        ethereum.admit_spend("commitment", &mut request, id).await?;
        assert_eq!(fs::read(&path)?, before);
        let changed = ethereum
            .api
            .raw_provider()
            .fill(
                request
                    .clone()
                    .with_gas_limit(request.gas.context("admitted gas missing")? + 1),
            )
            .await?;
        let changed = changed
            .as_envelope()
            .context("changed envelope unsigned")?
            .encoded_2718();
        assert!(ethereum
            .validate_admitted_spend("commitment", id, &changed)
            .await
            .is_err());
        assert_eq!(fs::read(&path)?, before);
        let receipt = ethereum.resume_commitment(&original).await?;
        assert!(receipt.status());
        ethereum
            .api
            .raw_provider()
            .client()
            .request::<_, Value>("anvil_mine", ("0x3",))
            .await?;
        let finalized = ethereum
            .api
            .get_finalized_receipt(receipt.transaction_hash)
            .await?
            .context("owned original not finalized")?;
        ethereum
            .finalize_commitment_spend(&original, &finalized)
            .await?;
        let completed = fs::read(&path)?;
        let ledger: Ledger = serde_json::from_slice(&completed)?;
        assert!(!ledger.reservations.contains_key(&id));
        assert_eq!(ledger.settled, maximum_cost);
        ethereum
            .finalize_commitment_spend(&original, &finalized)
            .await?;
        assert_eq!(fs::read(&path)?, completed);
        Ok(())
    }

    #[test]
    fn unresolved_prior_day_spend_is_not_reset_or_counted_twice() -> Result<()> {
        let mut ledger = Ledger {
            version: 1,
            chain_id: 1,
            genesis: B256::repeat_byte(1),
            sender: Address::repeat_byte(2),
            day: 4,
            settled: U256::from(20),
            reservations: BTreeMap::new(),
        };
        ledger.reservations.insert(
            B256::repeat_byte(3),
            Reservation {
                role: "root".into(),
                approval: B256::repeat_byte(4),
                request: TransactionRequest::default().with_nonce(7),
                maximum_cost: U256::from(30),
            },
        );
        assert_eq!(ledger.committed_cost()?, U256::from(50));
        ledger.advance_day(5)?;
        let restored: Ledger = serde_json::from_slice(&serde_json::to_vec(&ledger)?)?;
        assert_eq!(restored.committed_cost()?, U256::from(30));
        ledger.advance_day(5)?;
        assert_eq!(ledger.committed_cost()?, U256::from(30));
        assert!(ledger.advance_day(4).is_err());
        Ok(())
    }
}
