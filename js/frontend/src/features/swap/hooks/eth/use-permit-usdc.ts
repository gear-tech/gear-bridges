import { useMutation } from '@tanstack/react-query';
import { useMemo } from 'react';
import { hexToNumber, slice } from 'viem';
import { useConfig, useSignTypedData } from 'wagmi';
import { readContract } from 'wagmi/actions';

import { useTokens } from '@/context';
import { useEthAccount } from '@/hooks';
import { definedAssert } from '@/utils';

import { CONTRACT_ADDRESS, ERC20PERMIT_NONCES_ABI, ERC5267_ABI } from '../../consts';

const PERMIT_DURATION_SECONDS = 60 * 60;

const PERMIT_TYPES = {
  Permit: [
    { name: 'owner', type: 'address' },
    { name: 'spender', type: 'address' },
    { name: 'value', type: 'uint256' },
    { name: 'nonce', type: 'uint256' },
    { name: 'deadline', type: 'uint256' },
  ],
} as const;

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
      abi: ERC20PERMIT_NONCES_ABI,
      address: usdcToken.address,
      functionName: 'nonces',
      args: [ethAccount.address],
    });
  };

  const getDomain = async () => {
    definedAssert(usdcToken, 'USDC token');

    const [, name, version, chainId, verifyingContract] = await readContract(config, {
      abi: ERC5267_ABI,
      address: usdcToken.address,
      functionName: 'eip712Domain',
    });

    return { name, version, chainId, verifyingContract };
  };

  const permit = async (value: bigint) => {
    definedAssert(ethAccount.address, 'Account address');

    const [nonce, domain] = await Promise.all([getNonce(), getDomain()]);
    const timestampSeconds = Math.floor(Date.now() / 1000);
    const deadline = BigInt(timestampSeconds + PERMIT_DURATION_SECONDS);

    const message = {
      owner: ethAccount.address,
      spender: CONTRACT_ADDRESS.ERC20_MANAGER,
      value,
      nonce,
      deadline,
    };

    const signature = await signTypedDataAsync({
      types: PERMIT_TYPES,
      primaryType: 'Permit',
      domain,
      message,
    });

    const r = slice(signature, 0, 32);
    const s = slice(signature, 32, 64);
    const v = hexToNumber(slice(signature, 64, 65));

    return { deadline, r, s, v };
  };

  return { ...useMutation({ mutationKey: ['eth-permit-usdc'], mutationFn: permit }) };
}

export { usePermitUSDC };
