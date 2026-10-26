use super::*;

impl Ethereum {
    pub(crate) async fn connect_heartbeat(
        endpoint: &str,
        finality: &cli_utils::EthereumFinalityArgs,
        wallet: &Path,
        manifest: &Value,
        fixture: Option<&OwnedFixtureNetwork>,
        public_network: PublicEthereumNetwork,
    ) -> Result<Self> {
        let ethereum = Self::connect_addresses(
            endpoint,
            finality,
            Some(wallet),
            manifest,
            fixture,
            public_network,
        )
        .await?;
        // The client and adapter must already match approval. Queue installation and
        // root inventory are deliberately not prerequisites for client maintenance.
        verify_manifest_mode(&ethereum, manifest, false).await?;
        ethereum.verify_heartbeat_bindings().await?;
        Ok(ethereum)
    }

    pub(crate) async fn verify_heartbeat_bindings(&self) -> Result<()> {
        let view = self.api.verified_finalized_view().await?;
        let first = bound_source(self, self.api.raw_provider().clone(), view.block_id()).await?;
        let second = bound_source(
            self,
            self.api.raw_witness_provider()?.clone(),
            view.block_id(),
        )
        .await?;
        ensure!(first == second, "HOLD: heartbeat source bindings disagree");
        view.recheck(self.api.raw_provider()).await?;
        Ok(())
    }
}

async fn bound_source<P: Provider + Clone>(
    ethereum: &Ethereum,
    provider: P,
    block: BlockId,
) -> Result<B256> {
    let client = BeefyClient::new(ethereum.client_address, provider.clone());
    let adapter = VaraQueueRootVerifier::new(ethereum.verifier_address, provider);
    let source = client.sourceDomain().block(block).call().await?;
    let chain = ethereum.api.finality_identity()?.0;
    let domain = crate::bridge_domain(source.0, chain, ethereum.queue_address.into_array())?;
    ensure!(
        client.bridgeDomain().block(block).call().await? == B256::from(domain)
            && client.destinationChainId().block(block).call().await? == U256::from(chain)
            && client.destinationQueue().block(block).call().await? == ethereum.queue_address
            && client.mmrStartBlock().block(block).call().await? > 0
            && adapter.beefyClient().block(block).call().await? == ethereum.client_address
            && adapter.messageQueue().block(block).call().await? == ethereum.queue_address
            && adapter.destinationChainId().block(block).call().await? == U256::from(chain),
        "HOLD: heartbeat client/adapter immutable bindings changed"
    );
    Ok(source)
}
