import { useNetworkType } from '@/context/network-type';
import { deploymentOrigin } from '@/utils/deployment-origin';

function FaucetLink() {
  const { isTestnet } = useNetworkType();
  if (!isTestnet) return null;
  try {
    const origin = deploymentOrigin(import.meta.env.VITE_FAUCET_ORIGIN);
    if (origin === window.location.origin) return null;
    return (
      <a href={origin} target="_blank" rel="noopener noreferrer">
        Open separate test faucet
      </a>
    );
  } catch {
    return <span>Test faucet deployment is not configured.</span>;
  }
}

export { FaucetLink };
