import { useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { Sails } from 'sails-js';

function useSails(url: string | undefined) {
  const { api } = useApi();

  const getSails = async () => {
    if (!url) throw new Error('IDL URL is not found');
    if (!api) throw new Error('Api is not initialized');

    const response = await fetch(url);
    const idl = await response.text();
    const sails = await Sails.new();
    sails.parseIdl(idl);
    sails.setApi(api);

    return sails;
  };

  const { data } = useQuery({
    queryKey: ['sails', url],
    queryFn: getSails,
    enabled: Boolean(url && api),
  });

  return data;
}

export { useSails };
