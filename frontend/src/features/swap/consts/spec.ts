import bridgeMetadataUrl from '../assets/bridge_vara.meta.txt';
import ftBridgeMetadataUrl from '../assets/bridge_vara_wrapped_tokens.meta.txt';

const TOKEN_TYPE = {
  NATIVE: 'native',
  FUNGIBLE: 'fungible',
} as const;

const METADATA_URL = {
  [TOKEN_TYPE.NATIVE]: bridgeMetadataUrl,
  [TOKEN_TYPE.FUNGIBLE]: ftBridgeMetadataUrl,
} as const;

export { TOKEN_TYPE, METADATA_URL };
