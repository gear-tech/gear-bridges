import { useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { useAppKitNetwork } from '@reown/appkit/react';
import { useMutation } from '@tanstack/react-query';

import { useNetworkType } from '@/context/network-type';
import { logger } from '@/utils';

function SwitchEthWalletNetworkButton() {
  const { switchNetwork } = useAppKitNetwork();
  const { NETWORK_PRESET } = useNetworkType();
  const alert = useAlert();

  const { mutateAsync, isPending } = useMutation({
    mutationFn: () => switchNetwork(NETWORK_PRESET.ETH_NETWORK),
  });

  const handleClick = () => {
    mutateAsync().catch((error: Error) => {
      logger.error('Wallet network switch', error);
      alert.error(`Failed to switch wallet network. ${error.message}`);
    });
  };

  return <Button text="Switch Wallet Network" onClick={handleClick} isLoading={isPending} block />;
}

export { SwitchEthWalletNetworkButton };
