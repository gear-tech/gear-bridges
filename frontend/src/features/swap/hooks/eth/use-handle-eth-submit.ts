import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateFeesPerGas, estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { ERC20_MANAGER_ABI, ERC20_MANAGER_CONTRACT_ADDRESS, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';
import { InsufficientAccountBalanceError } from '../../errors';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';
import { useMint } from './use-mint';

const TRANSFER_GAS_LIMIT_FALLBACK = 21000n * 10n;

function useHandleEthSubmit(
  fee: bigint | undefined,
  allowance: bigint | undefined,
  ftBalance: bigint | undefined,
  accountBalance: bigint | undefined,
  openTransactionModal: (amount: string, receiver: string) => void,
) {
  const { token } = useBridgeContext();
  const { writeContractAsync } = useWriteContract();
  const mint = useMint();
  const approve = useApprove();
  const config = useConfig();

  const getTransferGasLimit = (amount: bigint, accountAddress: HexString) => {
    definedAssert(fee, 'Fee');
    definedAssert(token.address, 'Fungible token address');

    const encodedData = encodeFunctionData({
      abi: ERC20_MANAGER_ABI,
      functionName: 'requestBridgingPayingFee',
      args: [token.address, amount, accountAddress, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS],
    });

    return estimateGas(config, {
      to: ERC20_MANAGER_CONTRACT_ADDRESS,
      data: encodedData,
      value: fee,
    });
  };

  const validateBalance = async (amount: bigint, accountAddress: HexString) => {
    definedAssert(token.address, 'Fungible token address');
    definedAssert(fee, 'Fee');
    definedAssert(allowance, 'Allowance');
    definedAssert(ftBalance, 'Fungible token balance');
    definedAssert(accountBalance, 'Account balance');

    const isMintRequired = token.isNative && amount > ftBalance;
    const valueToMint = isMintRequired ? amount - ftBalance : BigInt(0);
    const mintGasLimit = isMintRequired ? await mint.getGasLimit(valueToMint) : BigInt(0);

    const isApproveRequired = amount > allowance;
    const approveGasLimit = isApproveRequired ? await approve.getGasLimit(amount) : BigInt(0);

    // if approve is not made, transfer gas estimate will fail.
    // it can be avoided by using stateOverride,
    // but it requires the knowledge of the storage slot or state diff of the allowance for each token,
    // which is not feasible to do programmatically (at least I didn't managed to find a convenient way to do so).
    const transferGasLimit = isApproveRequired ? undefined : await getTransferGasLimit(amount, accountAddress);

    // TRANSFER_GAS_LIMIT_FALLBACK is just for balance check, during the actual transfer it will be recalculated
    const gasLimit = mintGasLimit + approveGasLimit + (transferGasLimit || TRANSFER_GAS_LIMIT_FALLBACK);

    const { maxFeePerGas } = await estimateFeesPerGas(config);
    const weiGasLimit = gasLimit * maxFeePerGas;

    const balanceToWithdraw = valueToMint + weiGasLimit + fee;

    if (balanceToWithdraw > accountBalance) throw new InsufficientAccountBalanceError('ETH', balanceToWithdraw);

    return { valueToMint, isMintRequired, isApproveRequired, mintGasLimit, approveGasLimit, transferGasLimit };
  };

  const transfer = async (amount: bigint, accountAddress: HexString, gasLimit: bigint | undefined) => {
    definedAssert(token.address, 'Fungible token address');
    definedAssert(fee, 'Fee');

    const hash = await writeContractAsync({
      abi: ERC20_MANAGER_ABI,
      address: ERC20_MANAGER_CONTRACT_ADDRESS,
      functionName: 'requestBridgingPayingFee',
      args: [token.address, amount, accountAddress, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS],
      value: fee,
      gas: gasLimit,
    });

    return waitForTransactionReceipt(config, { hash });
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    const { valueToMint, isMintRequired, isApproveRequired, mintGasLimit, approveGasLimit, transferGasLimit } =
      await validateBalance(amount, accountAddress);

    openTransactionModal(amount.toString(), accountAddress);

    if (isMintRequired) {
      await mint.mutateAsync({ value: valueToMint, gas: mintGasLimit });
    } else {
      mint.reset();
    }

    if (isApproveRequired) {
      await approve.mutateAsync({ amount, gas: approveGasLimit });
    } else {
      approve.reset();
    }

    return transfer(amount, accountAddress, transferGasLimit);
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, approve, undefined, mint] as const;
}

export { useHandleEthSubmit };
