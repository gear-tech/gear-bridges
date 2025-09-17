import { HexString } from '@gear-js/api';
import { useConfig } from 'wagmi';
import { readContract } from 'wagmi/actions';

import { ERC20_ABI } from '@/consts';
import { useEthAccount } from '@/hooks';
import { definedAssert } from '@/utils';

import { CONTRACT_ADDRESS } from '../../consts';

function useGetEthAllowance(address: HexString | undefined) {
  const ethAccount = useEthAccount();
  const config = useConfig();

  return (accountOverride: HexString | undefined) => {
    const accountAddress = ethAccount.address || accountOverride;

    definedAssert(address, 'FT address');
    definedAssert(accountAddress, 'Allowance account address');

    return readContract(config, {
      address,
      abi: ERC20_ABI,
      functionName: 'allowance',
      args: [accountAddress, CONTRACT_ADDRESS.ERC20_MANAGER],
    });
  };
}

export { useGetEthAllowance };
