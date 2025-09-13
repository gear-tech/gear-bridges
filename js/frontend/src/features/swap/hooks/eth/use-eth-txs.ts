import { useQuery } from '@tanstack/react-query';

import { useEthAccount } from '@/hooks';
import { definedAssert, isUndefined } from '@/utils';

import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';
import { useMint } from './use-mint';
import { usePermitUSDC } from './use-permit-usdc';
import { useTransfer } from './use-transfer';

const TRANSFER_GAS_LIMIT_FALLBACK = 21000n * 10n;

type Transaction = {
  call: () => Promise<unknown>;
  gasLimit: bigint;
  value?: bigint;
};

type Params = {
  formValues: FormattedValues | undefined;
  allowance: bigint | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
};

function useEthTxs({ allowance, bridgingFee, shouldPayBridgingFee, formValues }: Params) {
  const ethAccount = useEthAccount();

  const { token } = useBridgeContext();

  const mint = useMint();
  const approve = useApprove();
  const permitUSDC = usePermitUSDC();

  const { transferWithoutFee, transferWithFee } = useTransfer(bridgingFee);
  const transfer = shouldPayBridgingFee ? transferWithFee : transferWithoutFee;

  const getTxs = async () => {
    definedAssert(allowance, 'Allowance');
    definedAssert(bridgingFee, 'Fee');
    definedAssert(token, 'Fungible token');

    const ALICE_ACCOUNT_ADDRESS = '0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d';
    const DUMMY_ETH_ADDRESS = '0x000000000000000000000000000000000000dEaD';

    const amount = formValues ? formValues.amount : 1n;
    const accountAddress = formValues ? formValues.accountAddress : ALICE_ACCOUNT_ADDRESS;
    const accountOverride = ethAccount.address ? undefined : DUMMY_ETH_ADDRESS;

    const txs: Transaction[] = [];
    const shouldMint = token.isNative;
    const shouldApprove = amount > allowance;
    const isUSDC = token.symbol.toLowerCase().includes('usdc');

    if (shouldMint) {
      const value = amount;
      const gasLimit = await mint.getGasLimit({ value, accountOverride });

      txs.push({
        call: () => mint.mutateAsync({ value }),
        gasLimit,
        value,
      });
    }

    let permit: Awaited<ReturnType<typeof permitUSDC.mutateAsync>> | undefined;

    if (shouldApprove) {
      if (isUSDC) {
        permit = await permitUSDC.mutateAsync(amount);
      } else {
        const call = () => approve.mutateAsync({ amount });
        const gasLimit = await approve.getGasLimit({ amount, accountOverride });

        txs.push({ call, gasLimit });
      }
    }

    // if approve is not made, transfer gas estimate will fail.
    // it can be avoided by using stateOverride,
    // but it requires the knowledge of the storage slot or state diff of the allowance for each token,
    // which is not feasible to do programmatically (at least I didn't managed to find a convenient way to do so).
    txs.push({
      call: () => transfer.mutateAsync({ amount, accountAddress, permit }),

      gasLimit: shouldApprove
        ? TRANSFER_GAS_LIMIT_FALLBACK
        : await transfer.getGasLimit({ amount, accountAddress, accountOverride }),

      value: shouldPayBridgingFee ? bridgingFee : undefined,
    });

    return txs;
  };

  return useQuery({
    queryKey: [
      'eth-txs',
      allowance?.toString(),
      bridgingFee?.toString(),
      shouldPayBridgingFee,
      formValues?.amount.toString(),
      formValues?.accountAddress,
      ethAccount?.address,
      token,
    ],

    queryFn: getTxs,

    // it's probably worth to check isConnecting too, but there is a bug:
    // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
    enabled: Boolean(!isUndefined(allowance) && !isUndefined(bridgingFee) && token && !ethAccount.isReconnecting),
  });
}

export { useEthTxs };
