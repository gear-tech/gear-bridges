use gear_core::ids::prelude::ActorIdExt;

use super::*;

alloy::sol! {
    #[sol(rpc)]
    interface CutoverAdmission {
        function paused() external view returns (bool);
    }
}

/// Read-only feasibility check. No nonce, signature, custody transfer, or exclusion is created.
pub(super) async fn preflight_liabilities(
    api: &GearApi,
    eth: &PollingEthApi,
    path: &Path,
) -> Result<()> {
    use vft_client::traits::{Vft as _, VftAdmin as _};
    let mut work = read_work(path)?;
    let policy = work.intent.coverage_policy.clone().context(
        "HOLD: legacy work has no independently approved deployment/feasibility policy; preserve it")?;
    let view = eth.verified_finalized_view().await?;
    let address = alloy_primitives::Address::from_slice(policy.erc20_manager.as_bytes());
    let at = alloy::rpc::types::BlockId::hash_canonical(view.block_hash());
    let primary = CutoverAdmission::new(address, &**eth)
        .paused()
        .block(at)
        .call()
        .await?;
    let witness = CutoverAdmission::new(address, eth.raw_witness_provider()?)
        .paused()
        .block(at)
        .call()
        .await?;
    ensure!(
        primary && witness,
        "HOLD: ERC20Manager deposit/payment admission must be paused before coverage cutover"
    );
    view.recheck(&**eth).await?;
    ensure!(
        work.blocks
            .first()
            .is_some_and(|block| block.number == policy.deployment_block)
            && work
                .blocks
                .last()
                .is_some_and(|block| block.hash == work.intent.boundary.execution_hash),
        "HOLD: full deployment-to-boundary archive inventory required before BeginCutover"
    );
    policy.validate_deployment(&work.blocks[0])?;
    let at = api.last_block_hash().await?;
    let pin = sails_rs::H256::from_slice(at.as_ref());
    for (token, _, _) in &work.intent.mappings {
        let admin = vft_client::VftAdmin::new(GClientRemoting::new(api.clone()));
        ensure!(
            !admin.is_paused().at_block(pin).recv(actor(*token)).await?,
            "HOLD: token {token:?} is paused; source remains usable before BeginCutover"
        );
        vft_client::Vft::new(GClientRemoting::new(api.clone()))
            .balance_of(actor(work.intent.old_identity.vft_manager_address))
            .at_block(pin)
            .recv(actor(*token))
            .await
            .context("HOLD: token balance/cooperation query failed before BeginCutover")?;
    }
    let signer: gsdk::signer::Signer = api.clone().into();
    let scanner = sender(
        ApiProviderConnection::from_api(signer.api().clone()),
        &work.intent.old_identity,
        Some(work.intent.erc20_manager),
        &work.intent,
    )?;
    let originals = seal::read_original_journal(&work.intent.old_transactions)?;
    for index in 0..work.admissions.len() {
        if work.admissions[index].proof.is_empty() {
            continue;
        }
        let admission = &work.admissions[index];
        let qualified: Result<()> = async {
            if let Some(original) = originals.transactions.values()
                .find(|row| row.tx.tx_hash == admission.original_tx_hash)
            {
                ensure!(!matches!(original.status,
                    relayer::message_relayer::eth_to_gear::tx_manager::TxStatus::Unprocessable { .. }),
                    "HOLD: original journal retains an unprocessable liability");
            }
            let block = work.blocks.iter().find(|block| block.hash == admission.original_block_hash)
                .context("HOLD: liability has no authenticated block")?;
            let receipt = block.receipts.iter().find(|receipt| receipt.transaction_hash == admission.original_tx_hash)
                .context("HOLD: liability receipt unavailable")?;
            ensure!(explicit_success(receipt), "HOLD: liability has no explicit successful receipt status");
            for log in receipt.inner.logs() {
                if log.address().as_slice() != work.intent.erc20_manager.as_bytes()
                    || log.inner.data.topics().first() != Some(&IERC20Manager::BridgingRequested::SIGNATURE_HASH) { continue; }
                let event = IERC20Manager::BridgingRequested::decode_log(&log.inner)?.data;
                let mapping = work.intent.mappings.iter().find(|(_, token, _)| token.as_bytes() == event.token.as_slice())
                    .context("HOLD: unmapped deposit is a retained liability, not a refund")?;
                ensure!(event.to != B256::ZERO && !event.amount.is_zero(),
                    "HOLD: invalid deposit is a retained liability, not an exclusion");
                if Some(mapping.0) == work.intent.native_wrapper {
                    ensure!(event.to.0 != ActorId::SYSTEM.into_bytes()
                        && event.amount < (alloy_primitives::U256::from(1) << 80),
                        "HOLD: impossible native payout remains a liability");
                }
            }
            let mut bytes = admission.proof.as_slice();
            let proof = eth_events_electra_client::EthToVaraEvent::decode(&mut bytes)?;
            ensure!(bytes.is_empty(), "HOLD: trailing proof bytes");
            if proof.receipt_rlp != ethereum_common::utils::map_receipt_envelope(receipt.as_ref()).encoded_2718() {
                let original = admission.signed.as_ref()
                    .or_else(|| admission.external_original.as_ref().map(|external| &external.original))
                    .or_else(|| originals.transactions.values().chain(originals.completed.values())
                        .find(|row| row.tx.tx_hash == admission.original_tx_hash)
                        .and_then(|row| row.receipt.as_ref())
                        .and_then(|receipt| receipt.signed_submission.as_ref()))
                    .context("HOLD: historical receipt has no exact retained original")?;
                scanner.authenticate_retained_receipt_framing(api, &proof, original).await?;
            }
            // Settlement can satisfy the liability, never erase or rewrite an error original.
            if matches!(scanner.settlement_status(api, &proof, None).await?,
                (MessageStatus::Success { receipt_observation: Some(_) }, _)) { return Ok(()); }
            let payload = receipt_call(&scanner, &proof);
            // Reserve a conservative 1 KiB for the complete signed Gear extrinsic envelope.
            let envelope_size = payload.len().checked_add(1024).context("Envelope size overflow")?;
            ensure!(envelope_size <= usize::try_from(policy.max_redirect_bytes)?
                && envelope_size <= 2_621_440,
                "HOLD: complete Redirect/extrinsic envelope too large ({envelope_size} bytes)");
            let gas = api.calculate_handle_gas_at(None,
                actor(work.intent.old_identity.historical_proxy_address), payload, 0, false, Some(at)).await?;
            ensure!(gas.min_limit <= policy.max_gas && gas.min_limit <= api.block_gas_limit()?,
                "HOLD: composed receipt exceeds approved Gear gas budget");
            Ok(())
        }.await;
        // Keep every raw block, proof, original and reason. A later candidate is still inspected.
        if let Some(previous) = work.admissions[index].unprocessable.take() {
            work.admissions[index].liability_history.push(previous);
        }
        if let Err(error) = qualified {
            work.admissions[index].unprocessable = Some(format!("retained liability: {error:#}"));
        }
    }
    ensure!(
        api.get_block_hash(api.block_number_at(at).await?).await? == at,
        "HOLD: preflight finalized pin changed"
    );
    save(path, &work)?;
    ensure_processable(&work.admissions)
}

pub(super) fn ensure_processable(admissions: &[Admission]) -> Result<()> {
    ensure!(
        admissions
            .iter()
            .all(|admission| !admission.proof.is_empty() && admission.unprocessable.is_none()),
        "HOLD: unprocessable/unmapped liabilities retained; BeginCutover forbidden (VB-19/VB-25)"
    );
    Ok(())
}
