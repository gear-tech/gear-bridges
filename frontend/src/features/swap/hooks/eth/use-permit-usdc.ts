import { useMutation } from '@tanstack/react-query';
import { useMemo } from 'react';
import { hexToNumber, slice } from 'viem';
import { useConfig, useSignTypedData } from 'wagmi';
import { readContract } from 'wagmi/actions';

import { ETH_CHAIN_ID } from '@/consts';
import { useTokens } from '@/context';
import { useEthAccount } from '@/hooks';
import { definedAssert } from '@/utils';

import { ERC20_MANAGER_CONTRACT_ADDRESS, USDC_ABI } from '../../consts';

const PERMIT_DURATION_SECONDS = 60 * 60;

function usePermitUSDC() {
  const ethAccount = useEthAccount();
  const { signTypedDataAsync } = useSignTypedData();
  const config = useConfig();

  const { tokens } = useTokens();
  const usdcToken = useMemo(
    () => tokens.eth?.find(({ symbol }) => symbol.toLowerCase().includes('usdc')),
    [tokens.eth],
  );

  const getNonce = () => {
    definedAssert(usdcToken, 'USDC token');
    definedAssert(ethAccount.address, 'Account address');

    return readContract(config, {
      abi: USDC_ABI,
      address: usdcToken.address,
      functionName: 'nonces',
      args: [ethAccount.address],
    });
  };

  const permit = async (value: bigint) => {
    definedAssert(usdcToken, 'USDC token');
    definedAssert(ethAccount.address, 'Account address');

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
      name: usdcToken.name,
      version: '1', // hardcoded for now, should be fetched from the contract but for now there's no such query
      chainId: ETH_CHAIN_ID,
      verifyingContract: usdcToken.address,
    };

    const timestampSeconds = Math.floor(Date.now() / 1000);
    const deadline = BigInt(timestampSeconds + PERMIT_DURATION_SECONDS);

    const message = {
      owner: ethAccount.address,
      spender: ERC20_MANAGER_CONTRACT_ADDRESS,
      value,
      nonce: await getNonce(),
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

  return { ...useMutation({ mutationFn: permit }) };
}

export { usePermitUSDC };
