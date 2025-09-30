import { useAccount, useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';

import { Tooltip } from '@/components';
import { useEthAccount, useModal } from '@/hooks';
import { getErrorMessage, isUndefined } from '@/utils';

import { usePayVaraFee, useVaraFee } from '../../hooks';

type Props = {
  nonce: string;
  onInBlock: () => void;
  onFinalization: () => void;
};

function PayVaraFeeButton({ nonce, onInBlock, onFinalization }: Props) {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const alert = useAlert();

  const { bridgingFee } = useVaraFee();
  const { sendTransactionAsync, isPending } = usePayVaraFee();

  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();

  const handlePayFeeButtonClick = () => {
    if (!account) return openSubstrateModal();
    if (isUndefined(bridgingFee.value)) throw new Error('Fee is not found');

    sendTransactionAsync({ args: [nonce], value: bridgingFee.value })
      .then(({ isFinalized }) => {
        alert.success('Fee paid successfully');
        onInBlock();

        return isFinalized;
      })
      .then(() => onFinalization())
      .catch((error: Error) => alert.error(getErrorMessage(error)));
  };

  const renderTooltipText = () => (
    <>
      <p>Pay a small fee in Vara chain tokens to let the bridge automatically deliver assets.</p>
      <p>Recommended if you don&apos;t have tokens to pay gas on the Ethereum chain.</p>
      {!account && <p>Wallet connection will be requested.</p>}
    </>
  );

  if (!account && !ethAccount.address) return;

  return (
    <>
      <Tooltip value={renderTooltipText()}>
        <Button
          text="Claim Automatically"
          size="x-small"
          onClick={handlePayFeeButtonClick}
          isLoading={isUndefined(bridgingFee.value) || isPending}
        />
      </Tooltip>

      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { PayVaraFeeButton };
