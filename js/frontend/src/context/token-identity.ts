import { GearApi } from '@gear-js/api';
import {
  authenticateInboundSource,
  immutableInboundProfile,
  nativeAssetKind,
  validateTokenPairInventory,
  type InboundProofProfile,
} from '@gear-js/bridge';

import { NetworkEnum, type Pair } from '@/features/history/graphql/graphql';
import { SailsProgram } from '@/features/swap/consts/sails/vft-manager';

export function validateActivePairs(pairs: readonly Pair[], mappings: readonly [string, string, string][]) {
  validateTokenPairInventory(
    pairs.map((pair) => ({
      ...pair,
      tokenSupply:
        pair.tokenSupply === NetworkEnum.Vara
          ? 'Gear'
          : pair.tokenSupply === NetworkEnum.Ethereum
            ? 'Ethereum'
            : pair.tokenSupply,
    })),
    mappings,
  );
}

export async function authenticateTokenPairs(api: GearApi, supplied: InboundProofProfile, pairs: readonly Pair[]) {
  const profile = immutableInboundProfile(supplied);
  const pin = (await api.blocks.getFinalizedHead()).toHex();
  await authenticateInboundSource(api, profile, pin);
  const program = await api.programStorage.getProgram(profile.consumer.programId, pin);
  if (!program.state.isInitialized || !program.codeId.eq(profile.consumer.codeId))
    throw new Error('HOLD: manager code differs from approval');
  const manager = new SailsProgram(api, profile.consumer.programId).vftManager;
  const [mappings, wrapper, paused, stopped, config, lifecycle] = await Promise.all([
    manager.varaToEthAddresses().atBlock(pin).call(),
    manager.nativeWrapper().atBlock(pin).call(),
    manager.isPaused().atBlock(pin).call(),
    manager.isEmergencyStopped().atBlock(pin).call(),
    manager.getConfig().atBlock(pin).call(),
    manager.cutoverStatus().atBlock(pin).call(),
  ]);
  validateActivePairs(pairs, mappings);
  if (profile.nativeWrapper) {
    const approved = profile.nativeWrapper;
    const deployed = await api.programStorage.getProgram(approved.programId, pin);
    if (
      !wrapper ||
      wrapper.actor_id.toLowerCase() !== approved.programId ||
      wrapper.ethereum_token.toLowerCase() !== approved.ethereumToken ||
      wrapper.code_id.toLowerCase() !== approved.codeId ||
      wrapper.mode !== (approved.mode === 'escrow-v1' ? 'EscrowV1' : 'HeldEscrowV2') ||
      !deployed.state.isInitialized ||
      !deployed.codeId.eq(approved.codeId)
    )
      throw new Error('HOLD: native VARA wrapper binding differs from approval');
  }
  if (profile.nativeEth) {
    const deployed = await api.programStorage.getProgram(profile.nativeEth.varaToken, pin);
    if (!deployed.state.isInitialized || !deployed.codeId.eq(profile.nativeEth.varaCodeId))
      throw new Error('HOLD: native ETH destination code differs from approval');
  }
  return {
    pin,
    config,
    profile,
    admissionOpen: !paused && !stopped && lifecycle.phase === 'Active' && BigInt(lifecycle.minimum_receipt_slot) > 0n,
  };
}

export function pairNativeKind(profile: InboundProofProfile | undefined, pair: Pair) {
  return nativeAssetKind(
    profile,
    pair.varaToken,
    pair.ethToken,
    pair.tokenSupply === NetworkEnum.Vara ? 'Gear' : 'Ethereum',
  );
}
