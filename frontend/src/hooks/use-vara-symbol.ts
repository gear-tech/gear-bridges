import { useApi } from '@gear-js/react-hooks';

function useVaraSymbol() {
  const { api } = useApi();

  return api?.registry.chainTokens[0];
}

export { useVaraSymbol };
