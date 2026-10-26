import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { WRAPPED_ETH_ABI } from '@/consts';
import { useNetworkType } from '@/context/network-type';
import { definedAssert } from '@/utils';

import { useBridgeContext } from '../../context';
import { assertSuccessfulReceipt } from '../../submission';

function useMint() {
  const { NETWORK_PRESET } = useNetworkType();
  const { token } = useBridgeContext();
  const { writeContractAsync } = useWriteContract();
  const config = useConfig();

  const mint = async ({ value }: { value: bigint }) => {
    definedAssert(token?.address, 'Fungible token address');

    const hash = await writeContractAsync({
      chainId: NETWORK_PRESET.ETH_CHAIN_ID,
      abi: WRAPPED_ETH_ABI,
      address: token.address, // only for wrapped eth
      functionName: 'deposit',
      value,
    });

    return assertSuccessfulReceipt(await waitForTransactionReceipt(config, { hash }), hash);
  };

  const getGasLimit = ({ value }: { value: bigint }) => {
    definedAssert(token?.address, 'Fungible token address');

    const data = encodeFunctionData({
      abi: WRAPPED_ETH_ABI,
      functionName: 'deposit',
    });

    return estimateGas(config, { to: token.address, data, value });
  };

  return { ...useMutation({ mutationFn: mint }), getGasLimit };
}

export { useMint };
