import { useAlert } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { useQueryClient } from '@tanstack/react-query';

import { Tooltip } from '@/components';
import { getErrorMessage, isUndefined } from '@/utils';

import { usePayVaraFee, useVaraFee } from '../../hooks';

type Props = {
  transactionId: string;
  nonce: string;
};

function PayVaraFeeButton({ transactionId, nonce }: Props) {
  const queryClient = useQueryClient();
  const alert = useAlert();

  const { bridgingFee } = useVaraFee();
  const { sendTransactionAsync, isPending } = usePayVaraFee();

  const handlePayFeeButtonClick = () => {
    if (isUndefined(bridgingFee.value)) throw new Error('Fee is not found');

    sendTransactionAsync({ args: [nonce], value: bridgingFee.value })
      .then(() => {
        alert.success('Fee paid successfully');

        return queryClient.invalidateQueries({ queryKey: ['transaction', transactionId] });
      })
      .catch((error: Error) => alert.error(getErrorMessage(error)));
  };

  const renderTooltipText = () => (
    <>
      <p>Pay a small fee in Vara chain tokens to let the bridge automatically deliver your assets.</p>
      <p>Recommended if you don&apos;t have tokens to pay gas on the Ethereum chain.</p>
    </>
  );

  return (
    <Tooltip value={renderTooltipText()}>
      <Button
        text="Claim Automatically"
        size="x-small"
        onClick={handlePayFeeButtonClick}
        isLoading={isUndefined(bridgingFee.value) || isPending}
      />
    </Tooltip>
  );
}

export { PayVaraFeeButton };
