import { Account, PublicClient, WalletClient } from 'viem';
import { GearApi, HexString } from '@gear-js/api';

import { getMessageQueueClient } from '../ethereum/index.js';
import { GearClient } from '../vara/index.js';
import { messageHash } from './util.js';
import { StatusCb } from '../util.js';

/**
 * Relays a message from Vara network to Ethereum by finding the queued message,
 * fetching merkle proof, and processing it through the message queue contract.
 *
 * @param nonce - The message nonce to relay (little endian encoded if hex string)
 * @param blockNumber - The Vara block number containing the message sent by EthBridge builtin
 * @param ethereumPublicClient - Ethereum public client for reading blockchain state
 * @param ethereumWalletClient - Ethereum wallet client for sending transactions
 * @param ethereumAccount - Ethereum account to use for transactions
 * @param gearApi - Gear API instance for interacting with Vara network
 * @param messageQueueuAddress - Ethereum message queue contract address
 * @param silent - Whether to suppress logging output
 * @returns Promise that resolves when the message is successfully processed
 */
export async function relayVaraToEth(
  nonce: bigint | HexString,
  blockNumber: bigint,
  ethereumPublicClient: PublicClient,
  ethereumWalletClient: WalletClient,
  ethereumAccount: Account,
  gearApi: GearApi,
  messageQueueuAddress: `0x${string}`,
  statusCb?: StatusCb,
) {
  if (!statusCb) {
    statusCb = () => {};
  }

  const gearClient = new GearClient(gearApi);
  const msgQClient = getMessageQueueClient(
    messageQueueuAddress,
    ethereumPublicClient,
    ethereumWalletClient,
    ethereumAccount,
  );

  let blockHash = (await gearApi.blocks.getBlockHash(Number(blockNumber))).toHex();

  if (typeof nonce === 'string') {
    nonce = BigInt(nonce);
  }

  statusCb(`Fetching message from block`, { nonce: nonce.toString(), blockHash });
  const msg = await gearClient.findMessageQueuedEvent(blockHash, nonce);

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

    const merkleRootFromLogs = await msgQClient.findMerkleRootInRangeOfBlocks(from, to, blockNumber);

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
  const merkleProof = await gearClient.fetchMerkleProof(blockHash, msgHash);

  return msgQClient.processMessage(blockNumber, msg, merkleProof, statusCb);
}
