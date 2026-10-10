import { HexString } from '@gear-js/api';
import { immutableInboundProfile, type InboundProofProfile } from '@gear-js/bridge';

import { IDL_SHA256 } from '@/features/swap/consts/sails/vft-manager';

import { getNetworkEnv } from '../utils';

const NODE_ADDRESS = getNetworkEnv('VARA_NODE_ADDRESSES');
const ARCHIVE_NODE_ADDRESS = getNetworkEnv('VARA_ARCHIVE_NODE_ADDRESSES');

const ETH_NODE_ADDRESS = getNetworkEnv('ETH_NODE_ADDRESSES');
const ETH_BEACON_NODE_ADDRESS = getNetworkEnv('ETH_BEACON_NODE_ADDRESSES');
const ETH_CHAIN_ID = getNetworkEnv('ETH_CHAIN_IDS', (value) => Number(value));

const INDEXER_ADDRESS = getNetworkEnv('INDEXER_ADDRESSES');

const BRIDGING_PAYMENT_CONTRACT_ADDRESS = getNetworkEnv<HexString>('BRIDGING_PAYMENT_CONTRACT_ADDRESSES');
const VFT_MANAGER_CONTRACT_ADDRESS = getNetworkEnv<HexString>('VFT_MANAGER_CONTRACT_ADDRESSES');

const ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS = getNetworkEnv<HexString>('ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESSES');
const ERC20_MANAGER_CONTRACT_ADDRESS = getNetworkEnv<HexString>('ERC20_MANAGER_CONTRACT_ADDRESSES');

const ETH_MESSAGE_QUEUE_CONTRACT_ADDRESS = getNetworkEnv<HexString>('ETH_MESSAGE_QUEUE_CONTRACT_ADDRESSES');

type SerializedInboundProofProfile = Omit<InboundProofProfile, 'ethereumChainId' | 'beaconGenesisTime' | 'forks'> & {
  ethereumChainId: string;
  beaconGenesisTime: string;
  forks: Array<{ name: string; epoch: string; version: HexString }>;
};
function approvedInboundProfile(network: 'MAINNET' | 'TESTNET'): { profile?: InboundProofProfile; hold?: string } {
  try {
    const raw = import.meta.env.VITE_INBOUND_PROOF_PROFILES as string | undefined;
    if (!raw) return { hold: 'HOLD: independently approved inbound deployment profile is unavailable.' };
    const supplied = (JSON.parse(raw) as Partial<Record<'MAINNET' | 'TESTNET', SerializedInboundProofProfile>>)[
      network
    ];
    if (!supplied) return { hold: 'HOLD: this network has no approved inbound deployment profile.' };
    const profile = immutableInboundProfile({
      ...supplied,
      ethereumChainId: BigInt(supplied.ethereumChainId),
      beaconGenesisTime: BigInt(supplied.beaconGenesisTime),
      forks: supplied.forks.map((fork) => ({ ...fork, epoch: BigInt(fork.epoch) })),
    });
    if (
      profile.ethereumChainId !== BigInt(ETH_CHAIN_ID[network]) ||
      profile.consumer.programId.toLowerCase() !== VFT_MANAGER_CONTRACT_ADDRESS[network].toLowerCase() ||
      profile.consumer.idlSha256 !== IDL_SHA256 ||
      profile.consumer.service !== 'VftManager' ||
      profile.consumer.method !== 'SubmitReceipt' ||
      !profile.nativeWrapper
    ) {
      return { hold: 'HOLD: inbound consumer/profile differs from this network deployment.' };
    }
    return { profile };
  } catch {
    return { hold: 'HOLD: inbound deployment profile is malformed or unknown.' };
  }
}
const INBOUND_PROOF_PROFILE = {
  MAINNET: approvedInboundProfile('MAINNET'),
  TESTNET: approvedInboundProfile('TESTNET'),
};

export {
  NODE_ADDRESS,
  ARCHIVE_NODE_ADDRESS,
  ETH_NODE_ADDRESS,
  ETH_BEACON_NODE_ADDRESS,
  ETH_CHAIN_ID,
  INDEXER_ADDRESS,
  BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  VFT_MANAGER_CONTRACT_ADDRESS,
  ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  ERC20_MANAGER_CONTRACT_ADDRESS,
  ETH_MESSAGE_QUEUE_CONTRACT_ADDRESS,
  INBOUND_PROOF_PROFILE,
};
