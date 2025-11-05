import { useAlert } from '@gear-js/react-hooks';
import { Button, Modal } from '@gear-js/vara-ui';
import { useAppKitNetwork } from '@reown/appkit/react';
import { useMutation } from '@tanstack/react-query';

import { logger } from '@/utils';

import { useNetworkType } from './context';

function UnsupportedNetworkModal() {
  const ethNetwork = useAppKitNetwork();
  const alert = useAlert();
  const { NETWORK_PRESET } = useNetworkType();

  // const isOpen = ethNetwork.chainId !== NETWORK_PRESET.ETH_CHAIN_ID;
  const isOpen = false;

  const { mutateAsync, isPending } = useMutation({
    mutationFn: () => ethNetwork.switchNetwork(NETWORK_PRESET.ETH_NETWORK),
  });

  const handleClick = () => {
    mutateAsync().catch((error: Error) => {
      alert.error(`Failed to switch network. ${error.message}`);
      logger.error('Network switch', error);
    });
  };

  if (!isOpen) return;

  return (
    <Modal heading="Network Mismatch" className="unsupportedNetworkModal" close={() => {}}>
      <p style={{ fontSize: '12px', textAlign: 'center', marginBottom: '4px' }}>
        You are connected to the wrong Ethereum network.
      </p>
      <p style={{ fontSize: '12px', textAlign: 'center', marginBottom: '32px' }}>
        Switch to {NETWORK_PRESET.ETH_NETWORK.name} to match the selected network type.
      </p>

      <Button text="Switch" size="x-small" isLoading={isPending} onClick={handleClick} block />
    </Modal>
  );
}

export { UnsupportedNetworkModal };
