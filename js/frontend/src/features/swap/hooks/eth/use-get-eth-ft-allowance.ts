import { HexString } from '@gear-js/api';
import { useConfig } from 'wagmi';
import { readContract } from 'wagmi/actions';

import { ERC20_ABI } from '@/consts';
import { useNetworkType } from '@/context/network-type';
import { useEthAccount } from '@/hooks';
import { definedAssert } from '@/utils';

function useGetEthAllowance(address: HexString | undefined) {
  const { NETWORK_PRESET } = useNetworkType();
  const ethAccount = useEthAccount();
  const config = useConfig();

  return () => {
    definedAssert(address, 'FT address');
    definedAssert(ethAccount.address, 'Ethereum account address');

    return readContract(config, {
      address,
      abi: ERC20_ABI,
      functionName: 'allowance',
      args: [ethAccount.address, NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS],
    });
  };
}

export { useGetEthAllowance };
