import { GearApi, HexString } from '@gear-js/api';
import {
  relayEthToVara,
  createVftManagerReplyContract,
  type InboundTokenEffect,
  type RelayResult,
} from '@gear-js/bridge';
import { useAccount } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';
import { usePublicClient } from 'wagmi';

import { useNetworkType } from '@/context/network-type';
import { definedAssert } from '@/utils';

import { SailsProgram, IDL_SHA256 } from '../../consts/sails/vft-manager';
import { createNativeRedemptionReader } from '../../native-redemption';
import { useHistoricalProxyContractAddress, useInitArchiveApi } from '../vara';

type Params = {
  onLog: (message: string) => void;
  onFinalized: (result: RelayResult) => void;
  onError: (error: Error) => void;
};

function useRelayEthTx(txHash: HexString, expectedEffect: InboundTokenEffect) {
  const { NETWORK_PRESET } = useNetworkType();
  const { account } = useAccount();
  const publicClient = usePublicClient();
  const { data: historicalProxyContractAddress } = useHistoricalProxyContractAddress();
  const initArchiveApi = useInitArchiveApi();

  const relay = async ({ onLog, onFinalized, onError }: Params) => {
    const deadline = Date.now() + 44 * 60 * 1000;
    let archiveApi: GearApi | undefined;

    try {
      definedAssert(account, 'Account');
      definedAssert(publicClient, 'Ethereum Public Client');
      definedAssert(historicalProxyContractAddress, 'Historical Proxy Contract Address');
      const profile = NETWORK_PRESET.INBOUND_PROOF_PROFILE.profile;
      if (!profile) throw new Error(NETWORK_PRESET.INBOUND_PROOF_PROFILE.hold);
      if (
        profile.ethereumChainId !== BigInt(NETWORK_PRESET.ETH_CHAIN_ID) ||
        profile.consumer.programId.toLowerCase() !== NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS.toLowerCase() ||
        expectedEffect.managerAddress.toLowerCase() !== NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS.toLowerCase()
      ) {
        throw new Error('HOLD: approved profile and configured token lane disagree');
      }

      archiveApi = await initArchiveApi();

      const manager = new SailsProgram(archiveApi, profile.consumer.programId);
      const readRedemption = createNativeRedemptionReader(archiveApi, profile);
      const result = await relayEthToVara({
        transactionHash: txHash,
        beaconRpcUrl: NETWORK_PRESET.ETH_BEACON_NODE_ADDRESS,
        ethereumPublicClient: publicClient,
        gearApi: archiveApi,
        historicalProxyId: historicalProxyContractAddress,
        inboundProfile: profile,
        consumerReply: createVftManagerReplyContract(profile, {
          registry: manager.registry,
          idlSha256: IDL_SHA256,
          expectedEffect,
          readDeposits: (pin, proof) =>
            manager.vftManager.receiptDeposits(proof.proofBlock.block.slot, proof.transactionIndex).atBlock(pin).call(),
          readRedemption,
          readReceiptStatus: (pin, proof) =>
            manager.vftManager.receiptStatus(proof.proofBlock.block.slot, proof.transactionIndex).atBlock(pin).call(),
        }),
        clientId: NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS,
        clientServiceName: 'VftManager',
        clientMethodName: 'SubmitReceipt',
        signer: account.decodedAddress,
        signerOptions: { signer: account.signer },
        statusCb: onLog,
        wait: true,
        deadline,
      });

      onFinalized(result);
    } catch (error) {
      onError(error as Error);
    } finally {
      await archiveApi?.disconnect();
    }
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayEthTx };
