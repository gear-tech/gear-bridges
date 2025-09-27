import { Account, PublicClient, WalletClient } from 'viem';
import { GearApi, HexString } from '@gear-js/api';

import { getMessageQueueClient } from '../ethereum/index.js';
import { GearClient } from '../vara/index.js';
import { messageHash } from './util.js';
import { StatusCb } from '../util.js';

/**
 * Parameters for relaying a Vara network message to Ethereum.
 * This interface defines all the required configuration and optional settings
 * needed to relay cross-chain messages from Vara to Ethereum.
 */
export type RelayVaraToEthParams = {
  /**
   * The message nonce to relay (bigint or hex string, little endian encoded if hex)
   */
  nonce: bigint | HexString;
  /**
   * The Vara block number containing the initial transaction
   */
  blockNumber: bigint;
  /**
   * Viem public client for reading Ethereum blockchain state
   */
  ethereumPublicClient: PublicClient;
  /**
   * Viem wallet client for sending Ethereum transactions
   */
  ethereumWalletClient: WalletClient;
  /**
   * Ethereum account to use for transaction signing and sending
   */
  ethereumAccount: Account;
  /**
   * Gear API instance for interacting with the Vara network
   */
  gearApi: GearApi;
  /**
   * Address of the Ethereum message queue contract
   */
  messageQueueAddress: `0x${string}`;
  /**
   * If true, waits for MerkleRoot to appear on MessageQueue contract instead of throwing error
   */
  wait?: boolean;
  /**
   * Optional callback function to track relay operation status
   */
  statusCb?: StatusCb;
};

/**
 * Relays a message from the Vara network to Ethereum through the bridge infrastructure.
 *
 * This function performs the complete relay process:
 * 1. Fetches the queued message from the Vara network using the provided nonce and block
 * 2. Retrieves or waits for the merkle root to appear in the Ethereum message queue contract
 * 3. Generates a merkle proof for the message
 * 4. Processes the message through the Ethereum message queue contract
 *
 * @param params - Configuration parameters for the relay operation
 * @param params.nonce - The message nonce to relay (bigint or hex string, little endian encoded if hex)
 * @param params.blockNumber - The Vara block number containing the message sent by EthBridge builtin
 * @param params.ethereumPublicClient - Viem public client for reading Ethereum blockchain state
 * @param params.ethereumWalletClient - Viem wallet client for sending Ethereum transactions
 * @param params.ethereumAccount - Ethereum account to use for transaction signing and sending
 * @param params.gearApi - Gear API instance for interacting with the Vara network
 * @param params.messageQueueAddress - Address of the Ethereum message queue contract
 * @param params.wait - If true, waits for MerkleRoot to appear on MessageQueue contract instead of throwing error
 * @param params.statusCb - Optional callback function to track relay operation status
 *
 * @returns Promise resolving to message processing result with transaction details and status
 *
 * @throws {Error} When message with specified nonce is not found in the block
 * @throws {Error} When wallet client is not provided for transaction signing
 * @throws {Error} When merkle proof generation fails
 * @throws {Error} When Ethereum transaction submission fails
 *
 * @example
 * ```typescript
 * const result = await relayVaraToEth({
 *   nonce: 123n,
 *   blockNumber: 456789n,
 *   ethereumPublicClient: publicClient,
 *   ethereumWalletClient: walletClient,
 *   ethereumAccount: account,
 *   gearApi: gearApi,
 *   messageQueueAddress: '0x123...',
 *   wait: true,
 *   statusCb: (status, details) => console.log(status, details)
 * });
 *
 * if (result.success) {
 *   console.log('Message relayed successfully:', result.transactionHash);
 * } else {
 *   console.error('Relay failed:', result.error);
 * }
 * ```
 */
export async function relayVaraToEth(params: RelayVaraToEthParams) {
  const {
    ethereumPublicClient,
    ethereumWalletClient,
    ethereumAccount,
    gearApi,
    messageQueueAddress,
    wait = false,
    statusCb = () => {},
  } = params;

  let nonce = params.nonce;
  let blockNumber = params.blockNumber;
  const gearClient = new GearClient(gearApi);
  const msgQClient = getMessageQueueClient(
    messageQueueAddress,
    ethereumPublicClient,
    ethereumWalletClient,
    ethereumAccount,
  );

  let blockHash = (await gearApi.blocks.getBlockHash(Number(blockNumber))).toHex();

  if (typeof nonce === 'string') {
    nonce = BigInt(nonce);
  }

  statusCb(`Fetching message from block`, { nonce: nonce.toString(), blockHash });
  const msg = await gearClient.findMessageQueuedEvent(Number(blockNumber), nonce);

  if (!msg) {
    throw new Error(`Message with nonce ${nonce} not found in block ${blockNumber}`);
  }

  const authoritySetId = await gearClient.getAuthoritySetIdByBlockNumber(blockNumber);
  statusCb(`Authority set ID for block ${blockNumber}: ${authoritySetId}`);

  statusCb(`Fetching merkle root`, { blockNumber: blockNumber.toString() });
  let merkleRoot = await msgQClient.getMerkleRoot(blockNumber);

  if (!merkleRoot) {
    statusCb(`Merkle root not found. Looking for suitable submitted merkle root`);
    const gearBlockTimestamp = (await gearApi.blocks.getBlockTimestamp(blockHash)).toNumber();
    const ethereumHead = await ethereumPublicClient.getBlock();
    const ethereumHeadTimestamp = Number(ethereumHead.timestamp);

    const diff = ethereumHeadTimestamp - gearBlockTimestamp / 1000;

    const [from, to] = [ethereumHead.number - BigInt(Math.floor(diff / 12)), ethereumHead.number];

    statusCb(`Requesting MerkleRoot logs`, { fromBlock: from.toString(), toBlock: to.toString() });

    const merkleRootFromLogs = wait
      ? await msgQClient.waitForMerkleRoot(blockNumber, from, statusCb)
      : await msgQClient.findMerkleRootInRangeOfBlocks(from, to, blockNumber);

    if (merkleRootFromLogs.blockNumber === blockNumber) {
      merkleRoot = merkleRootFromLogs.merkleRoot;
      statusCb(`Merkle root received`, { blockNumber: blockNumber.toString(), merkleRoot });
    } else {
      const authoritySetIdForClosestBlock = await gearClient.getAuthoritySetIdByBlockNumber(
        merkleRootFromLogs.blockNumber,
      );

      statusCb(`Merkle root received for a different block`, {
        blockNumber: merkleRootFromLogs.blockNumber.toString(),
        merkleRoot: merkleRootFromLogs.merkleRoot,
      });

      if (authoritySetIdForClosestBlock === authoritySetId) {
        merkleRoot = merkleRootFromLogs.merkleRoot;
        blockNumber = merkleRootFromLogs.blockNumber;
        blockHash = (await gearApi.blocks.getBlockHash(Number(blockNumber))).toHex();
      }
    }
  }

  const msgHash = messageHash(msg);
  statusCb(`Fetching merkle proof`, { blockNumber: blockNumber.toString(), msgHash });
  const merkleProof = await gearClient.fetchMerkleProof(Number(blockNumber), msgHash);

  return msgQClient.processMessage(blockNumber, msg, merkleProof, statusCb);
}
