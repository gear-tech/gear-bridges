import { useAlert } from '@gear-js/react-hooks';
import { Button, Modal } from '@gear-js/vara-ui';
import { useAppKitNetwork } from '@reown/appkit/react';
import { useMutation } from '@tanstack/react-query';
import { useChainId, useSwitchChain } from 'wagmi';

import { logger } from '@/utils';

import { useNetworkType } from './context';

function UnsupportedNetworkModal() {
  const appKitNetwork = useAppKitNetwork();
  const wagmiChainId = useChainId();
  const { switchChainAsync: switchWagmiNetwork } = useSwitchChain();
  const alert = useAlert();
  const { NETWORK_PRESET, isMainnet } = useNetworkType();

  const isMismatch =
    appKitNetwork.chainId !== NETWORK_PRESET.ETH_CHAIN_ID ||
    wagmiChainId !== NETWORK_PRESET.ETH_CHAIN_ID ||
    appKitNetwork.chainId !== wagmiChainId;

  const { mutateAsync, isPending } = useMutation({
    mutationFn: () =>
      Promise.all([
        switchWagmiNetwork({ chainId: NETWORK_PRESET.ETH_CHAIN_ID }),
        appKitNetwork.switchNetwork(NETWORK_PRESET.ETH_NETWORK),
      ]),
  });

  const handleClick = () => {
    mutateAsync().catch((error: Error) => {
      alert.error(`Failed to switch network. ${error.message}`);
      logger.error('Network switch', error);
    });
  };

  if (!isMismatch) return;

  return (
    <Modal heading="Network Mismatch" className="unsupportedNetworkModal" close={() => {}}>
      <p style={{ fontSize: '12px', textAlign: 'center', marginBottom: '4px' }}>
        You are connected to different Ethereum network.
      </p>
      <p style={{ fontSize: '12px', textAlign: 'center', marginBottom: '32px' }}>
        Switch to {NETWORK_PRESET.ETH_NETWORK.name}
        {isMainnet ? ' Mainnet' : ' Testnet'} to match the selected network type.
      </p>

      <Button text="Switch" size="x-small" isLoading={isPending} onClick={handleClick} block />
    </Modal>
  );
}

export { UnsupportedNetworkModal };
