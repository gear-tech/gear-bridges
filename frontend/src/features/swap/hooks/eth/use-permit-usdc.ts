import { useMutation } from '@tanstack/react-query';
import { hexToNumber, slice } from 'viem';
import { useReadContract, useSignTypedData } from 'wagmi';

import { ETH_CHAIN_ID } from '@/consts';
import { useTokens } from '@/context';
import { useEthAccount } from '@/hooks';
import { definedAssert, isUndefined } from '@/utils';

import { ERC20_MANAGER_CONTRACT_ADDRESS, USDC_ABI } from '../../consts';

const PERMIT_DURATION_SECONDS = 60 * 60;

function usePermitUSDC() {
  const ethAccount = useEthAccount();
  const { signTypedDataAsync } = useSignTypedData();

  const { tokens } = useTokens();
  const { address } = tokens.active?.find(({ symbol }) => symbol.toLowerCase().includes('usdc')) || {};

  const { data: version } = useReadContract({
    abi: USDC_ABI,
    address,
    functionName: 'version',
  });

  const { data: name } = useReadContract({
    abi: USDC_ABI,
    address,
    functionName: 'name',
  });

  const { data: nonce } = useReadContract({
    abi: USDC_ABI,
    address,
    functionName: 'nonces',
    args: ethAccount.address ? [ethAccount.address] : undefined,
  });

  const permit = async (value: bigint) => {
    definedAssert(ethAccount.address, 'Account address');
    definedAssert(address, 'USDC address');
    definedAssert(version, 'USDC version');
    definedAssert(name, 'USDC name');
    definedAssert(nonce, 'USDC nonce');

    const types = {
      Permit: [
        { name: 'owner', type: 'address' },
        { name: 'spender', type: 'address' },
        { name: 'value', type: 'uint256' },
        { name: 'nonce', type: 'uint256' },
        { name: 'deadline', type: 'uint256' },
      ],
    };

    const domain = {
      name,
      version,
      chainId: ETH_CHAIN_ID,
      verifyingContract: address,
    };

    const timestampSeconds = Math.floor(Date.now() / 1000);
    const deadline = BigInt(timestampSeconds + PERMIT_DURATION_SECONDS);

    const message = {
      owner: ethAccount.address,
      spender: ERC20_MANAGER_CONTRACT_ADDRESS,
      value,
      nonce,
      deadline,
    };

    const signature = await signTypedDataAsync({
      types,
      primaryType: 'Permit',
      domain,
      message,
    });

    const r = slice(signature, 0, 32);
    const s = slice(signature, 32, 64);
    const v = hexToNumber(slice(signature, 64, 65));

    return { deadline, r, s, v };
  };

  const isLoading = !version || !name || isUndefined(nonce);

  return { ...useMutation({ mutationFn: permit }), isLoading };
}

export { usePermitUSDC };
