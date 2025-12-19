import { useAccount, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';

import { Tooltip } from '@/components';
import { useModal } from '@/hooks';
import { getErrorMessage, isUndefined } from '@/utils';

import { usePayVaraFee, useVaraFee } from '../../hooks';

type Props = {
  nonce: string;
  onInBlock: () => void;
};

function PayVaraFeeButton({ nonce, onInBlock }: Props) {
  const { account } = useAccount();

  const alert = useAlert();

  const { bridgingFee } = useVaraFee();
  const { sendTransactionAsync, isPending } = usePayVaraFee();

  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();

  const handlePayFeeButtonClick = () => {
    if (!account) return openSubstrateModal();
    if (isUndefined(bridgingFee)) throw new Error('Fee is not found');

    sendTransactionAsync({ args: [nonce], value: bridgingFee })
      .then(() => {
        alert.success('Fee paid successfully');
        onInBlock();
      })
      .catch((error: Error) => alert.error(getErrorMessage(error)));
  };

  const renderTooltipText = () => (
    <>
      <p>Pay a small fee in Vara chain tokens to let the bridge automatically deliver assets.</p>
      <p>Recommended if you don&apos;t have tokens to pay gas on the Ethereum chain.</p>
      {!account && <p>Wallet connection will be requested.</p>}
    </>
  );

  if (!account) return;

  return (
    <>
      <Tooltip value={renderTooltipText()}>
        <Button
          text="Claim Automatically"
          size="x-small"
          onClick={handlePayFeeButtonClick}
          isLoading={isUndefined(bridgingFee) || isPending}
        />
      </Tooltip>

      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { PayVaraFeeButton };
