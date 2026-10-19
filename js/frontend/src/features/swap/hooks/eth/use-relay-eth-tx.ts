import { GearApi, HexString } from '@gear-js/api';
import {
  relayEthToVara,
  validateInboundTokenEffect,
  validateNativeTokenEffect,
  type InboundTokenEffect,
  type NativeRedemption,
  type RelayResult,
} from '@gear-js/bridge';
import { useAccount } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';
import { usePublicClient } from 'wagmi';

import { SailsProgram as NativeProgram } from '@/consts/sails/vft-vara';
import { useNetworkType } from '@/context/network-type';
import { definedAssert } from '@/utils';

import { SailsProgram, IDL_SHA256 } from '../../consts/sails/vft-manager';
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
        profile.consumer.programId !== NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS ||
        expectedEffect.managerAddress !== NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS
      ) {
        throw new Error('HOLD: approved profile and configured token lane disagree');
      }

      archiveApi = await initArchiveApi();

      const manager = new SailsProgram(archiveApi, profile.consumer.programId);
      const nativeWrapper = profile.nativeWrapper
        ? new NativeProgram(archiveApi, profile.nativeWrapper.programId)
        : undefined;
      const result = await relayEthToVara({
        transactionHash: txHash,
        beaconRpcUrl: NETWORK_PRESET.ETH_BEACON_NODE_ADDRESS,
        ethereumPublicClient: publicClient,
        gearApi: archiveApi,
        historicalProxyId: historicalProxyContractAddress,
        inboundProfile: profile,
        consumerReply: {
          registry: manager.registry,
          resultType: 'Result<Null, Error>',
          idlSha256: IDL_SHA256,
          nativeSettlement: profile.nativeWrapper
            ? {
                expectedEffect,
                readDeposits: (pin, proof) =>
                  manager.vftManager
                    .receiptDeposits(proof.proofBlock.block.slot, proof.transactionIndex)
                    .atBlock(pin)
                    .call(),
                readRedemption: (operationId, pin) =>
                  nativeWrapper!.nativeEscrow.redemption(operationId).atBlock(pin).call(),
                readReceiptStatus: (pin, proof) =>
                  manager.vftManager
                    .receiptStatus(proof.proofBlock.block.slot, proof.transactionIndex)
                    .atBlock(pin)
                    .call(),
              }
            : undefined,
          verifyEffect: async (_value, settlementResult, proof) => {
            const deposits = await manager.vftManager
              .receiptDeposits(proof.proofBlock.block.slot, proof.transactionIndex)
              .atBlock(settlementResult.effectBlockHash ?? settlementResult.replyBlockHash)
              .call();
            if (deposits.some((deposit) => deposit.native)) {
              definedAssert(nativeWrapper, 'HOLD: native wrapper identity is unavailable');
              definedAssert(profile.nativeWrapper, 'HOLD: native wrapper profile is unavailable');
              const identity = {
                managerId: profile.consumer.programId,
                proxyId: profile.historicalProxyId,
                wrapperId: profile.nativeWrapper.programId,
                slot: BigInt(proof.proofBlock.block.slot),
                transactionIndex: BigInt(proof.transactionIndex),
                receiptRlp: proof.receiptRlp,
                expectedEffect,
              };
              const redemptions = new Map<HexString, NativeRedemption | null>();
              for (const deposit of deposits.filter((entry) => entry.native)) {
                redemptions.set(
                  deposit.operation_id,
                  await nativeWrapper.nativeEscrow
                    .redemption(deposit.operation_id)
                    .atBlock(settlementResult.effectBlockHash ?? settlementResult.replyBlockHash)
                    .call(),
                );
              }
              definedAssert(
                settlementResult.nativeSettlements,
                'HOLD: authenticated original native settlements are missing',
              );
              const originals = new Map(
                settlementResult.nativeSettlements.map((entry) => [entry.operationId, entry.redemption]),
              );
              const settlements = validateNativeTokenEffect(identity, deposits, redemptions, originals);
              if (
                settlements.length !== settlementResult.nativeSettlements.length ||
                settlements.some(
                  (entry) =>
                    entry.child !==
                    settlementResult.nativeSettlements!.find((original) => original.operationId === entry.operationId)
                      ?.child,
                )
              )
                throw new Error('HOLD: original native settlement cohort changed');
            } else validateInboundTokenEffect(proof.receiptRlp, deposits, expectedEffect);
          },
        },
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
