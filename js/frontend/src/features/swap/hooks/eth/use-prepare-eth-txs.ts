import { definedAssert, isUndefined } from '@/utils';
import { useNetworkType } from '@/context/network-type';

import { SUBMIT_STATUS } from '../../consts';
import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';
import { useGetEthAllowance } from './use-get-eth-ft-allowance';
import { useMint } from './use-mint';
import { useTokenAdmission } from '../use-token-admission';
import { useTransfer } from './use-transfer';

const TRANSFER_GAS_LIMIT_FALLBACK = 21000n * 10n;

type Transaction = {
  call: () => Promise<unknown>;
  gasLimit: bigint;
  value?: bigint;
};

type Params = {
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  ftBalance: bigint | undefined;
};

function usePrepareEthTxs({ bridgingFee, shouldPayBridgingFee, ftBalance }: Params) {
  const { token } = useBridgeContext();
  const { NETWORK_PRESET } = useNetworkType();

  const getAllowance = useGetEthAllowance(token?.address);

  const mint = useMint();
  const approve = useApprove();
  const admit = useTokenAdmission(token);
  const transfer = useTransfer(bridgingFee, shouldPayBridgingFee);

  const prepare = async ({ amount, accountAddress }: FormattedValues) => {
    definedAssert(bridgingFee, 'Bridging fee');
    definedAssert(ftBalance, 'FT balance');
    definedAssert(token, 'Token');
    if (
      BigInt(accountAddress) === 0n ||
      (token.destinationAddress === NETWORK_PRESET.INBOUND_PROOF_PROFILE.profile?.nativeWrapper?.programId &&
        accountAddress.toLowerCase() === `0x${'67656172'.repeat(8)}`)
    )
      throw new Error('Invalid Vara recipient');

    const txs: Transaction[] = [];
    const shouldMint = token.isNative;

    const allowance = await getAllowance();
    const shouldApprove = amount > allowance;

    if (shouldMint) {
      const value = amount;
      const gasLimit = await mint.getGasLimit({ value });

      txs.push({
        call: async () => {
          await admit(token, { bridgingFee: shouldPayBridgingFee ? bridgingFee : undefined });
          return mint.mutateAsync({ value });
        },
        gasLimit,
        value,
      });
    }

    // No independently approved permit domain: use the selected token's exact-amount approval.
    if (shouldApprove) {
      const call = async () => {
        await admit(token, { bridgingFee: shouldPayBridgingFee ? bridgingFee : undefined });
        return approve.mutateAsync({ amount });
      };
      const gasLimit = await approve.getGasLimit({ amount });
      txs.push({ call, gasLimit });
    }

    // transfer estimate will fail if allowance or balance is less than amount.
    // we're checking balance at the form level, but not for WETH because it's always getting minted
    const canEstimateTransfer = !shouldApprove && ftBalance >= amount;

    txs.push({
      call: async () => {
        await admit(token, { bridgingFee: shouldPayBridgingFee ? bridgingFee : undefined });
        return transfer.mutateAsync({ amount, accountAddress });
      },

      gasLimit: canEstimateTransfer
        ? await transfer.getGasLimit({ amount, accountAddress })
        : TRANSFER_GAS_LIMIT_FALLBACK,

      value: shouldPayBridgingFee ? bridgingFee : undefined,
    });

    return txs;
  };

  const resetState = () => {
    mint.reset();
    approve.reset();
    transfer.reset();
  };

  const getStatus = () => {
    if (mint.isPending || mint.error) return SUBMIT_STATUS.MINT;
    if (approve.isPending || approve.error) return SUBMIT_STATUS.APPROVE;
    if (transfer.isPending || transfer.error) return SUBMIT_STATUS.BRIDGE;

    return SUBMIT_STATUS.SUCCESS;
  };

  return {
    prepare: !isUndefined(bridgingFee) && !!token ? prepare : undefined,
    resetState,
    status: getStatus(),
  };
}

export { usePrepareEthTxs };
