import { BridgeProvider, useBridgeContext } from '../../context';
import { SwapEthForm, SwapVaraForm } from '../swap-form';

function Component() {
  const { network } = useBridgeContext();
  const Form = network.isVara ? SwapVaraForm : SwapEthForm;

  return <Form />;
}

function Swap() {
  return (
    <BridgeProvider>
      <Component />
    </BridgeProvider>
  );
}

export { Swap };
