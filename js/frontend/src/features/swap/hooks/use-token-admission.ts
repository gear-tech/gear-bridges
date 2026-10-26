import { useApi } from '@gear-js/react-hooks';
import { useRef } from 'react';
import { keccak256, parseAbi } from 'viem';
import { usePublicClient } from 'wagmi';

import { useNetworkType } from '@/context/network-type';
import { authenticateTokenPairs } from '@/context/token-identity';
import { assertCurrentToken } from '@/context/token-selection';
import { useTokens, type Token } from '@/context/tokens';
import { usePairs } from '@/features/history';
import { useBridgingPaymentProgram } from './vara/use-bridging-payment-program';

const admissionAbi = parseAbi([
  'function paused() view returns (bool)',
  'function tokenAdmissionEnabled(address) view returns (bool)',
]);
const feeAbi = parseAbi(['function fee() view returns (uint256)']);

/** Re-read authority immediately before each wallet operation, not just when estimates were prepared. */
export function useTokenAdmission(selectedToken: Token | undefined, purpose: 'bridge' | 'unwrap' = 'bridge') {
  const { api } = useApi();
  const { NETWORK_PRESET } = useNetworkType();
  const client = usePublicClient();
  const { data: feeProgram } = useBridgingPaymentProgram();
  const { data: pairs } = usePairs();
  const { getActiveToken } = useTokens();
  const current = useRef({ api, NETWORK_PRESET, client, pairs, getActiveToken, feeProgram, selectedToken });
  current.current = { api, NETWORK_PRESET, client, pairs, getActiveToken, feeProgram, selectedToken };

  return async (token: Token | undefined, fees?: { bridgingFee?: bigint; managerFee?: bigint }) => {
    const started = Date.now();
    const start = current.current;
    const profile = start.NETWORK_PRESET.INBOUND_PROOF_PROFILE.profile;
    if (!token || !start.api || !start.client || !start.pairs || !profile || !token.isActive)
      throw new Error('HOLD: token capability is unavailable');
    const assertCurrent = () => assertCurrentToken(start, current.current, token);
    assertCurrent();
    const { config, pin, admissionOpen } = await authenticateTokenPairs(start.api, profile, start.pairs);
    if (purpose === 'bridge' && !admissionOpen)
      throw new Error('HOLD: manager admission is paused or outside its active replay domain');
    if (fees?.bridgingFee !== undefined && token.network === 'vara') {
      if (!start.feeProgram) throw new Error('HOLD: fee program unavailable');
      const state = await start.feeProgram.bridgingPayment.getState().atBlock(pin).call();
      if (BigInt(state.fee) !== fees.bridgingFee) throw new Error('HOLD: Vara bridging fee changed');
    }
    if (
      profile.consumer.programId !== start.NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS.toLowerCase() ||
      profile.ethereumChainId !== BigInt(start.NETWORK_PRESET.ETH_CHAIN_ID)
    )
      throw new Error('HOLD: configured lane differs from approval');
    if (fees?.managerFee !== undefined && BigInt(config.fee_incoming) !== fees.managerFee)
      throw new Error('HOLD: manager fee changed');
    const pinnedClient = start.client;
    const [chain, genesis, block] = await Promise.all([
      pinnedClient.getChainId(),
      pinnedClient.getBlock({ blockNumber: 0n }),
      pinnedClient.getBlock({ blockTag: 'finalized' }),
    ]);
    if (BigInt(chain) !== profile.ethereumChainId || genesis.hash?.toLowerCase() !== profile.ethereumGenesisHash)
      throw new Error('HOLD: Ethereum chain identity differs from approval');
    const ethereumToken = token.network === 'eth' ? token.address : token.destinationAddress;
    if (purpose === 'bridge') {
      const [paused, enabled] = await Promise.all([
        pinnedClient.readContract({
          address: start.NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS,
          abi: admissionAbi,
          functionName: 'paused',
          blockNumber: block.number,
        }),
        pinnedClient.readContract({
          address: start.NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS,
          abi: admissionAbi,
          functionName: 'tokenAdmissionEnabled',
          args: [ethereumToken],
          blockNumber: block.number,
        }),
      ]);
      if (paused || !enabled) throw new Error('HOLD: Ethereum token admission is disabled');
    }
    if (token.network === 'eth' && token.isNative) {
      if (
        !profile.nativeEth ||
        profile.nativeEth.ethereumToken !== token.address ||
        profile.nativeEth.varaToken !== token.destinationAddress
      )
        throw new Error('HOLD: native ETH capability not approved');
      const code = await pinnedClient.getBytecode({ address: token.address, blockNumber: block.number });
      if (!code || code === '0x' || keccak256(code) !== profile.nativeEth.runtimeCodeHash)
        throw new Error('HOLD: native ETH runtime code differs from approval');
    }
    if (fees?.bridgingFee !== undefined && token.network === 'eth') {
      const fee = await pinnedClient.readContract({
        address: start.NETWORK_PRESET.ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
        abi: feeAbi,
        functionName: 'fee',
        blockNumber: block.number,
      });
      if (fee !== fees.bridgingFee) throw new Error('HOLD: bridging fee changed');
    }
    if (Date.now() - started >= 30_000) throw new Error('HOLD: signing observations are stale');
    assertCurrent();
  };
}
