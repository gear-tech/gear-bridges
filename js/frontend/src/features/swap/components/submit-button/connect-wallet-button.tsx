import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import { useModal } from '@/hooks';

import { useBridgeContext } from '../../context';

function ConnectWalletButton() {
  const { network } = useBridgeContext();
  const { open: openEthModal } = useAppKit();
  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();

  const handleClick = () => {
    const openModal = network.isVara ? openSubstrateModal : openEthModal;

    void openModal();
  };

  return (
    <>
      <Button text="Connect Wallet" onClick={handleClick} block />
      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { ConnectWalletButton };
