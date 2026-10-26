'use strict';

// Run with Node 22 and NODE_PATH pointing at the existing root node_modules.
// This loopback service trusts the dedicated faucet nginx authentication scope.
// POSTs additionally require its exact independently configured Origin. Never
// run a second copy against these signers/state, or restore an older journal.
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { createHmac, timingSafeEqual, createHash } = require('node:crypto');
const assert = require('node:assert/strict');
const { decodeAddress, cryptoWaitReady } = require('@polkadot/util-crypto');
const { JsonRpcProvider, Wallet, Transaction, Interface, getAddress, keccak256, FetchRequest } = require('ethers');

const GENESIS = '0xa12f631344f61644239b70f9ab53e4842e540d8b0c1e7adb9570c06a62c3af3f';
const ETH_GENESIS = '0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b';
const TOKEN = '0xd4d65b1100b522cbde9664b9ba78c8add04e5b53';
const CODE_HASH = '0x6e3c691cf46703cdedaade8a407c2ee68e62f5094dd38e8f1278d4c0a5437e3d';
// Historical journal identity ONLY. Moving the HTTP origin must not rewrite it.
const ORIGIN = 'https://bridge-62-238-54-143.sslip.io';
const CHAIN_ID = 560048;
const COOLDOWN = 86400000;
const AMOUNTS = { TVARA: '100000000000000', USDC: '1000000000' };
const LIMITS = { TVARA: 10000000000000000n, USDC: 100000000000n };
const MAX_FEE = 2000000000n;
const ABI = new Interface([
  'function decimals() view returns(uint8)',
  'function balanceOf(address) view returns(uint256)',
  'function transfer(address,uint256) returns(bool)',
  'event Transfer(address indexed from,address indexed to,uint256 value)',
]);
const json = value => JSON.stringify(value);
const digest = value => createHash('sha256').update(value).digest('hex');
const hex32 = value => typeof value === 'string' && /^0x[0-9a-f]{64}$/.test(value);
function fail(message, code = 503, nextClaimAt) {
  return Object.assign(new Error(message), { publicMessage: message, code, nextClaimAt });
}
function normalize(asset, address) {
  if (typeof asset !== 'string' || !Object.hasOwn(AMOUNTS, asset) || typeof address !== 'string' || address.length > 128) throw fail('Invalid asset or address', 400);
  try {
    if (asset === 'USDC') {
      const result = getAddress(address);
      if (result === '0x0000000000000000000000000000000000000000') throw Error();
      return result.toLowerCase();
    }
    const bytes = decodeAddress(address);
    if (bytes.length !== 32 || bytes.every(x => x === 0)) throw Error();
    return '0x' + Buffer.from(bytes).toString('hex');
  } catch { throw fail('Invalid address', 400); }
}
async function bounded(promise, ms = 15000) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(fail('Network observation unavailable; existing original remains on hold')), ms); })]); }
  finally { clearTimeout(timer); }
}
function syncDir(directory) {
  const fd = fs.openSync(directory, 'r');
  try { fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
}
function privateFile(file, max = 16384) {
  const stat = fs.lstatSync(file);
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.size <= max && (stat.mode & 0o077) === 0, 'Private regular file required');
  return fs.readFileSync(file, 'utf8');
}

// Append-only authenticated records preserve originals and cumulative allocations.
// ponytail: replay is O(journal size), bounded by 200 authorized payouts; move
// scan progress to an authenticated checkpoint only if long HOLDs grow this log.
class Journal {
  constructor(directory, secret, identity) {
    this.directory = directory;
    this.secret = secret;
    this.identity = identity;
    this.seals = [];
    this.poisoned = false;
    this.initializing = !fs.existsSync(directory);
    fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
    const stat = fs.lstatSync(directory);
    assert(stat.isDirectory() && !stat.isSymbolicLink() && (stat.mode & 0o077) === 0, 'Private state directory required');
    syncDir(path.dirname(directory));
    this.load();
    if (!this.seals.length) this.append({ kind: 'identity', identity });
  }
  mac(body) { return createHmac('sha256', this.secret).update(json(body)).digest('hex'); }
  load() {
    assert(!this.poisoned, 'Journal is held after interrupted persistence');
    const files = fs.readdirSync(this.directory);
    const names = files.filter(name => name !== 'head.json').sort();
    let head;
    if (files.includes('head.json')) {
      head = JSON.parse(privateFile(path.join(this.directory, 'head.json')));
      assert(typeof head.mac === 'string' && /^[0-9a-f]{64}$/.test(head.mac));
      assert(timingSafeEqual(Buffer.from(head.mac, 'hex'), Buffer.from(this.mac(head.body), 'hex')), 'Journal head authentication failed');
    } else assert(this.initializing && names.length === 0, 'Missing journal head; HOLD, never initialize an existing directory');
    assert(names.every((name, i) => name === String(i).padStart(9, '0') + '.json'), 'Interrupted, missing or unexpected journal file; HOLD and preserve evidence');
    assert(names.length >= this.seals.length, 'Journal rollback detected');
    let previous = null;
    const claims = [];
    const seals = [];
    for (let i = 0; i < names.length; i++) {
      const text = privateFile(path.join(this.directory, names[i]), 65536);
      const { body, mac } = JSON.parse(text);
      assert(typeof mac === 'string' && /^[0-9a-f]{64}$/.test(mac));
      assert(timingSafeEqual(Buffer.from(mac, 'hex'), Buffer.from(this.mac(body), 'hex')), 'Journal authentication failed');
      assert(body.sequence === i && body.previous === previous, 'Journal chain mismatch');
      const seal = digest(text);
      if (this.seals[i]) assert.equal(seal, this.seals[i], 'Journal changed during ownership');
      const entry = body.entry;
      if (i === 0) {
        assert.equal(entry.kind, 'identity');
        assert.deepEqual(entry.identity, this.identity, 'Signer/config changed; HOLD');
      } else if (entry.kind === 'original') {
        const c = entry.claim;
        assert(Object.hasOwn(AMOUNTS, c.asset) && c.address === normalize(c.asset, c.address));
        assert.equal(c.amount, AMOUNTS[c.asset]);
        assert.equal(c.status, 'pending');
        assert(hex32(c.txHash) && /^0x(?:[0-9a-f]{2})+$/.test(c.raw) && c.raw.length < 32768);
        assert(Number.isSafeInteger(c.createdAt) && c.createdAt > 0 && c.nextClaimAt === c.createdAt + COOLDOWN);
        assert(Number.isSafeInteger(c.pin.height) && c.pin.height >= 0 && hex32(c.pin.hash));
        assert.equal(c.scanNext, c.pin.height);
        assert(!claims.some(x => x.asset === c.asset && x.status === 'pending'), 'Overlapping originals');
        const prior = claims.findLast(x => x.asset === c.asset && x.address === c.address);
        assert(!prior || c.createdAt >= prior.nextClaimAt, 'Cooldown violation');
        assert(!claims.some(x => x.txHash === c.txHash), 'Duplicate original');
        const priorNonce = claims.findLast(x => x.asset === c.asset)?.nonce;
        assert(typeof c.nonce === 'string' && /^\d+$/.test(c.nonce));
        assert.equal(BigInt(c.nonce), priorNonce === undefined ? 0n : BigInt(priorNonce) + 1n, 'Signer history gap; HOLD');
        claims.push(structuredClone(c));
      } else {
        const c = claims.find(x => x.txHash === entry.txHash);
        assert(c && c.status === 'pending', 'Unknown or settled original update');
        if (entry.kind === 'progress') {
          assert(Number.isSafeInteger(entry.next) && entry.next >= c.scanNext);
          c.scanNext = entry.next;
        } else {
          assert.equal(entry.kind, 'outcome');
          assert(['finalized', 'failed'].includes(entry.status) && hex32(entry.inclusion.hash));
          assert(Number.isSafeInteger(entry.inclusion.height) && entry.inclusion.height >= c.pin.height);
          c.status = entry.status;
          c.inclusion = entry.inclusion;
          if (c.status === 'failed') c.error = 'Finalized transaction failed; this daily claim and pool allocation remain consumed';
        }
      }
      previous = mac;
      seals.push(seal);
    }
    if (head) assert.deepEqual(head.body, { length: names.length, last: previous }, 'Journal head mismatch or tail rollback; HOLD');
    for (const asset of Object.keys(AMOUNTS)) assert(claims.filter(c => c.asset === asset).reduce((n, c) => n + BigInt(c.amount), 0n) <= LIMITS[asset], 'Pool allocation exceeded');
    this.seals = seals;
    this.previous = previous;
    this.claims = claims;
    return claims;
  }
  append(entry) {
    this.load();
    const body = { sequence: this.seals.length, previous: this.previous, entry };
    const text = json({ body, mac: this.mac(body) }) + '\n';
    const file = path.join(this.directory, String(body.sequence).padStart(9, '0') + '.json');
    const temporary = file + '.new';
    try {
      const fd = fs.openSync(temporary, 'wx', 0o600);
      try { fs.writeFileSync(fd, text); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
      assert(!fs.existsSync(file), 'Refusing to replace journal record');
      fs.renameSync(temporary, file);
      syncDir(this.directory);
      // Independent authenticated head detects deleted tail records. A crash
      // between these writes is deliberately a HOLD, never guessed recovery.
      const headBody = { length: body.sequence + 1, last: this.mac(body) };
      const headPath = path.join(this.directory, 'head.json');
      const headFd = fs.openSync(headPath + '.new', 'wx', 0o600);
      try { fs.writeFileSync(headFd, json({ body: headBody, mac: this.mac(headBody) }) + '\n'); fs.fsyncSync(headFd); } finally { fs.closeSync(headFd); }
      fs.renameSync(headPath + '.new', headPath);
      syncDir(this.directory);
      this.initializing = false;
      this.load();
    } catch (error) { this.poisoned = true; throw error; }
  }
}
function publicClaim(c) {
  if (!c) return { status: 'idle' };
  return { status: c.status, txHash: c.txHash, nextClaimAt: c.nextClaimAt, ...(c.error ? { error: c.error } : {}) };
}
function admission(claims, asset, address, now = Date.now()) {
  const normalized = normalize(asset, address);
  const previous = claims.findLast(c => c.asset === asset && c.address === normalized);
  if (previous?.status === 'pending') return previous;
  if (previous && now < previous.nextClaimAt) throw fail('Daily claim already consumed', 429, previous.nextClaimAt);
  if (claims.some(c => c.asset === asset && c.status === 'pending')) throw fail('Another original for this asset is unresolved; faucet is on hold');
  if (claims.filter(c => c.asset === asset).reduce((n, c) => n + BigInt(c.amount), 0n) + BigInt(AMOUNTS[asset]) > LIMITS[asset]) throw fail('Dedicated faucet pool allocation exhausted; no automatic refill');
  return null;
}
class Faucet {
  constructor(journal, chain) { this.journal = journal; this.chain = chain; this.jobs = new Map(); this.stopping = false; }
  async exclusive(asset, operation) {
    // Each signer has one operation at a time, including observation.
    while (this.jobs.has(asset)) await this.jobs.get(asset).catch(() => {});
    const job = Promise.resolve().then(operation);
    this.jobs.set(asset, job);
    try { return await job; } finally { if (this.jobs.get(asset) === job) this.jobs.delete(asset); }
  }
  status(asset, address) {
    const normalized = normalize(asset, address);
    return publicClaim(this.journal.load().findLast(c => c.asset === asset && c.address === normalized));
  }
  request(asset, address) {
    const normalized = normalize(asset, address);
    return this.exclusive(asset, async () => {
      if (this.stopping) throw fail('Faucet is shutting down');
      const claims = this.journal.load();
      const previous = admission(claims, asset, normalized);
      if (previous) return publicClaim(previous);
      // Preparation performs reads and local signing only. A timed-out prepare
      // may finish locally but has no authority to append or broadcast itself.
      const lastNonce = claims.findLast(c => c.asset === asset)?.nonce;
      const expectedNonce = lastNonce === undefined ? 0n : BigInt(lastNonce) + 1n;
      const prepared = await bounded(this.chain.prepare(asset, normalized, expectedNonce), 30000);
      if (this.stopping) throw fail('Faucet is shutting down');
      const createdAt = Date.now();
      const claim = { asset, address: normalized, amount: AMOUNTS[asset], status: 'pending', createdAt, nextClaimAt: createdAt + COOLDOWN, ...prepared, scanNext: prepared.pin.height };
      this.chain.validate(claim);
      this.journal.append({ kind: 'original', claim });
      // Exactly one attempt, only in the originating authorized HTTP operation.
      // On restart even a signed-but-never-sent original stays pending. No
      // rebroadcast, fresh nonce, timeout expiry or replacement is automatic.
      try { await bounded(this.chain.broadcast(claim)); } catch { /* uncertain original: observe only */ }
      return publicClaim(claim);
    });
  }
  observe(asset) {
    return this.exclusive(asset, async () => {
      const claim = this.journal.load().find(c => c.asset === asset && c.status === 'pending');
      if (!claim || this.stopping) return;
      this.chain.validate(claim);
      // Individual RPCs are bounded; a progressing 32-block scan may take longer.
      const update = await this.chain.observe(claim);
      if (update) this.journal.append({ ...update, txHash: claim.txHash });
    });
  }
}

function approvedOrigin(value) {
  assert(typeof value === 'string' && !/[\s*'"\\]/.test(value), 'Exact HTTPS origin required');
  const url = new URL(value);
  assert(url.protocol === 'https:' && !url.username && !url.password && value === url.origin &&
    /^(?:[a-z0-9.-]+|\[[0-9a-f:]+\])$/.test(url.hostname), 'Exact HTTPS origin required');
  return url;
}
function authorizeRequest(req, origin) {
  // nginx overwrites this header only after its own faucet-only Basic auth.
  if (typeof req.headers['x-faucet-authenticated'] !== 'string' || !req.headers['x-faucet-authenticated'])
    throw fail('Separate faucet authentication required', 403);
  if ((req.method === 'POST' && req.headers.origin !== origin) ||
      (req.headers.origin && req.headers.origin !== origin)) throw fail('Exact faucet Origin required', 403);
}
function configFromFile(file) {
  const c = JSON.parse(privateFile(file));
  const origin = approvedOrigin(c.origin), funds = approvedOrigin(c.fundsOrigin);
  assert(origin.hostname !== funds.hostname && c.origin !== ORIGIN, 'Separate approved faucet hostname required');
  assert(c.genesis === GENESIS && c.chainId === CHAIN_ID && c.ethereumGenesis === ETH_GENESIS && c.usdcContract === TOKEN && c.usdcCodeHash === CODE_HASH, 'Unapproved faucet configuration');
  assert(c.sourceRpc === 'ws://127.0.0.1:52545' && c.witnessRpc === 'ws://127.0.0.1:52547', 'Unapproved Gear endpoints');
  const eth = new URL(c.ethereumRpc);
  assert(eth.protocol === 'https:' && !eth.username && !eth.password, 'HTTPS Ethereum endpoint required');
  for (const key of ['stateDir', 'gearKeyFile', 'ethereumKeyFile']) assert(typeof c[key] === 'string' && path.isAbsolute(c[key]), 'Absolute state/key paths required');
  return c;
}
async function connect(c) {
  const { GearApi } = require('@gear-js/api');
  const { Keyring } = require('@polkadot/keyring');
  await cryptoWaitReady();
  const suri = privateFile(c.gearKeyFile).trim();
  // Known development authorities must never become faucet signers.
  assert(suri && !suri.startsWith('//'), 'Dedicated non-development Gear secret required');
  const gearKey = new Keyring({ type: 'sr25519' }).addFromUri(suri);
  const evmSecret = JSON.parse(privateFile(c.ethereumKeyFile)).private_key;
  const evmKey = new Wallet(evmSecret);
  const gearSender = normalize('TVARA', gearKey.address), evmSender = normalize('USDC', evmKey.address);
  assert(gearSender.slice(2, 42) !== evmSender.slice(2), 'Separate dedicated signers required');
  const fetchRequest = new FetchRequest(c.ethereumRpc);
  fetchRequest.timeout = 15000;
  const eth = new JsonRpcProvider(fetchRequest, undefined, { batchMaxCount: 1, cacheTimeout: -1 });
  const [source, witness] = await bounded(Promise.all([c.sourceRpc, c.witnessRpc].map(providerAddress => GearApi.create({ providerAddress }))), 30000);
  const rpc = (method, params) => bounded(eth.send(method, params));
  const quantity = n => '0x' + BigInt(n).toString(16);
  async function canonical(height) {
    const hashes = await bounded(Promise.all([source, witness].map(async api => (await api.rpc.chain.getBlockHash(height)).toHex())));
    assert.equal(hashes[0], hashes[1], 'Gear witness mismatch');
    return hashes[0];
  }
  async function gearPin() {
    const heads = await bounded(Promise.all([source, witness].map(async api => (await api.rpc.chain.getHeader(await api.rpc.chain.getFinalizedHead())).number.toNumber())));
    const height = Math.min(...heads);
    return { height, hash: await canonical(height) };
  }
  async function ethPin() {
    const block = await rpc('eth_getBlockByNumber', ['finalized', false]);
    assert(block && hex32(block.hash));
    return { height: Number(BigInt(block.number)), hash: block.hash };
  }
  async function ethCall(data, pin) {
    return rpc('eth_call', [{ to: TOKEN, data }, { blockHash: pin.hash, requireCanonical: true }]);
  }
  async function identity() {
    const geneses = await bounded(Promise.all([source, witness].map(api => api.rpc.chain.getBlockHash(0))));
    for (const genesis of geneses) assert.equal(genesis.toHex(), GENESIS);
    assert.equal(BigInt(await rpc('eth_chainId', [])), BigInt(CHAIN_ID));
    assert.equal((await rpc('eth_getBlockByNumber', ['0x0', false])).hash, ETH_GENESIS);
    const [gear, ethereum] = await Promise.all([gearPin(), ethPin()]);
    const code = await rpc('eth_getCode', [TOKEN, { blockHash: ethereum.hash, requireCanonical: true }]);
    assert.equal(keccak256(code), CODE_HASH);
    assert.equal(ABI.decodeFunctionResult('decimals', await ethCall(ABI.encodeFunctionData('decimals'), ethereum))[0], 6n);
    return { gear, ethereum };
  }
  function validate(claim) {
    assert.equal(claim.amount, AMOUNTS[claim.asset]);
    assert.equal(claim.address, normalize(claim.asset, claim.address));
    if (claim.asset === 'TVARA') {
      const tx = source.registry.createType('Extrinsic', claim.raw);
      assert(tx.isSigned && tx.era.isImmortalEra);
      assert.equal(tx.hash.toHex(), claim.txHash);
      assert.equal(normalize('TVARA', tx.signer.toString()), gearSender);
      assert.equal(tx.method.section, 'balances');
      assert.equal(tx.method.method, 'transferKeepAlive');
      assert.equal(normalize('TVARA', tx.method.args[0].toString()), claim.address);
      assert.equal(tx.method.args[1].toString(), AMOUNTS.TVARA);
      assert.equal(tx.nonce.toString(), claim.nonce);
    } else {
      const tx = Transaction.from(claim.raw);
      assert(tx.isSigned() && tx.type === 2 && tx.chainId === BigInt(CHAIN_ID));
      assert.equal(tx.hash, claim.txHash);
      assert.equal(tx.from.toLowerCase(), evmSender);
      assert.equal(tx.to.toLowerCase(), TOKEN);
      assert.equal(tx.value, 0n);
      assert(tx.gasLimit > 0n && tx.gasLimit <= 100000n && tx.maxFeePerGas > 0n && tx.maxFeePerGas <= MAX_FEE && tx.maxPriorityFeePerGas <= tx.maxFeePerGas);
      assert.equal(tx.nonce.toString(), claim.nonce);
      assert.equal(tx.data, ABI.encodeFunctionData('transfer', [claim.address, AMOUNTS.USDC]));
    }
  }
  return {
    secret: createHmac('sha256', evmKey.privateKey).update('isolated-faucet-journal-v1\0' + suri).digest(),
    identity: { version: 1, genesis: GENESIS, chainId: CHAIN_ID, ethereumGenesis: ETH_GENESIS, usdcContract: TOKEN, usdcCodeHash: CODE_HASH, origin: ORIGIN, gearSender, evmSender },
    identityCheck: identity,
    validate,
    async prepare(asset, address, expectedNonce) {
      const pins = await identity();
      if (asset === 'TVARA') {
        const pin = pins.gear;
        const account = await bounded(source.query.system.account.at(pin.hash, gearSender));
        const current = await bounded(source.query.system.account(gearSender));
        const next = await bounded(source.rpc.system.accountNextIndex(gearKey.address));
        assert.equal(next.toString(), account.nonce.toString(), 'Competing or unfinalized Gear nonce');
        assert.equal(current.nonce.toString(), account.nonce.toString());
        assert.equal(BigInt(next.toString()), expectedNonce, 'Gear signer history does not match journal; HOLD');
        const tx = source.tx.balances.transferKeepAlive(address, AMOUNTS.TVARA);
        const info = await bounded(tx.paymentInfo(gearKey, { nonce: next, era: 0 }));
        const data = account.data;
        const frozen = data.frozen?.toBigInt() ?? (data.feeFrozen?.toBigInt() > data.miscFrozen?.toBigInt() ? data.feeFrozen.toBigInt() : data.miscFrozen?.toBigInt() ?? 0n);
        const reserve = source.consts.balances.existentialDeposit.toBigInt() + frozen;
        if (data.free.toBigInt() < BigInt(AMOUNTS.TVARA) + info.partialFee.toBigInt() * 2n + reserve) throw fail('Dedicated TVARA account exhausted or locked; no automatic refill');
        await bounded(tx.signAsync(gearKey, { nonce: next, era: 0 }));
        return { pin, raw: tx.toHex(), txHash: tx.hash.toHex(), nonce: next.toString() };
      }
      const pin = pins.ethereum;
      const counts = await Promise.all(['pending', 'latest', quantity(pin.height)].map(tag => rpc('eth_getTransactionCount', [evmKey.address, tag])));
      assert(counts.every(n => BigInt(n) === BigInt(counts[0])), 'Competing or unfinalized Ethereum nonce');
      assert.equal(BigInt(counts[0]), expectedNonce, 'Ethereum signer history does not match journal; HOLD');
      const balance = ABI.decodeFunctionResult('balanceOf', await ethCall(ABI.encodeFunctionData('balanceOf', [evmKey.address]), pin))[0];
      if (balance < BigInt(AMOUNTS.USDC)) throw fail('Dedicated USDC account exhausted; no automatic refill');
      const data = ABI.encodeFunctionData('transfer', [address, AMOUNTS.USDC]);
      assert.equal(ABI.decodeFunctionResult('transfer', await rpc('eth_call', [{ from: evmKey.address, to: TOKEN, data, gas: quantity(100000) }, 'pending']))[0], true);
      const estimate = BigInt(await rpc('eth_estimateGas', [{ from: evmKey.address, to: TOKEN, data, gas: quantity(100000) }]));
      const gasLimit = (estimate * 120n + 99n) / 100n;
      if (gasLimit > 100000n || gasLimit === 0n) throw fail('USDC gas estimate exceeds approved bound');
      const latest = await rpc('eth_getBlockByNumber', ['latest', false]);
      const priority = 100000000n;
      const desired = BigInt(latest.baseFeePerGas) * 2n + priority;
      const maxFeePerGas = desired < MAX_FEE ? desired : MAX_FEE;
      if (BigInt(latest.baseFeePerGas) + priority > maxFeePerGas) throw fail('Hoodi fee exceeds 2 gwei maximum; no transaction signed');
      if (BigInt(await rpc('eth_getBalance', [evmKey.address, 'pending'])) < gasLimit * maxFeePerGas) throw fail('Dedicated Hoodi gas account exhausted; no automatic refill');
      const raw = await evmKey.signTransaction({ type: 2, chainId: CHAIN_ID, to: TOKEN, value: 0, data, nonce: Number(BigInt(counts[0])), gasLimit, maxFeePerGas, maxPriorityFeePerGas: priority });
      return { pin, raw, txHash: keccak256(raw), nonce: BigInt(counts[0]).toString() };
    },
    async broadcast(claim) {
      validate(claim);
      const hash = claim.asset === 'TVARA' ? (await bounded(source.rpc.author.submitExtrinsic(claim.raw))).toHex() : await rpc('eth_sendRawTransaction', [claim.raw]);
      assert.equal(hash, claim.txHash);
    },
    async observe(claim) {
      await identity();
      if (claim.asset === 'USDC') {
        assert.equal((await rpc('eth_getBlockByNumber', [quantity(claim.pin.height), false])).hash, claim.pin.hash);
        const receipt = await rpc('eth_getTransactionReceipt', [claim.txHash]);
        if (!receipt) return null;
        const head = await ethPin();
        const height = Number(BigInt(receipt.blockNumber));
        if (height > head.height) return null;
        const block = await rpc('eth_getBlockByNumber', [receipt.blockNumber, false]);
        assert.equal(block.hash, receipt.blockHash);
        assert(block.transactions.includes(claim.txHash) && receipt.transactionHash === claim.txHash);
        assert.equal(receipt.from.toLowerCase(), evmSender);
        assert.equal(receipt.to.toLowerCase(), TOKEN);
        assert(height >= claim.pin.height);
        let status = 'failed';
        if (BigInt(receipt.status) === 1n) {
          const transfers = receipt.logs.filter(log => log.address.toLowerCase() === TOKEN && log.topics[0] === ABI.getEvent('Transfer').topicHash).map(log => ABI.parseLog(log));
          assert.equal(transfers.length, 1, 'Unexpected USDC transfer effects; HOLD');
          assert.equal(transfers[0].args.from.toLowerCase(), evmSender);
          assert.equal(transfers[0].args.to.toLowerCase(), claim.address);
          assert.equal(transfers[0].args.value, BigInt(AMOUNTS.USDC));
          status = 'finalized';
        } else assert.equal(BigInt(receipt.status), 0n);
        return { kind: 'outcome', status, inclusion: { height, hash: block.hash } };
      }
      assert.equal(await canonical(claim.pin.height), claim.pin.hash);
      const end = await gearPin();
      // Bounded canonical scans resume from durable coverage, never timeouts.
      let next = claim.scanNext;
      for (; next <= Math.min(end.height, claim.scanNext + 31); next++) {
        const hash = await canonical(next);
        const block = await bounded(source.rpc.chain.getBlock(hash));
        assert.equal(block.block.header.hash.toHex(), hash);
        const index = block.block.extrinsics.findIndex(tx => tx.hash.toHex() === claim.txHash);
        if (index < 0) continue;
        assert.equal(block.block.extrinsics[index].toHex(), claim.raw);
        const [events, witnessEvents] = await bounded(Promise.all([source, witness].map(api => api.query.system.events.at(hash))));
        assert.equal(events.toHex(), witnessEvents.toHex(), 'Witness event mismatch');
        const own = events.filter(e => e.phase.isApplyExtrinsic && e.phase.asApplyExtrinsic.toNumber() === index).map(e => e.event);
        const success = own.filter(e => e.section === 'system' && e.method === 'ExtrinsicSuccess');
        const failure = own.filter(e => e.section === 'system' && e.method === 'ExtrinsicFailed');
        assert.equal(success.length + failure.length, 1, 'Dispatch outcome missing; HOLD');
        if (success.length) {
          const transfers = own.filter(e => e.section === 'balances' && e.method === 'Transfer');
          assert.equal(transfers.length, 1, 'Transfer event missing; HOLD');
          const [from, to, amount] = transfers[0].data;
          assert.equal(normalize('TVARA', from.toString()), gearSender);
          assert.equal(normalize('TVARA', to.toString()), claim.address);
          assert.equal(amount.toString(), AMOUNTS.TVARA);
        }
        return { kind: 'outcome', status: success.length ? 'finalized' : 'failed', inclusion: { height: next, hash } };
      }
      return next > claim.scanNext ? { kind: 'progress', next } : null;
    },
    close: () => Promise.allSettled([source.disconnect(), witness.disconnect(), Promise.resolve().then(() => eth.destroy())]),
  };
}

async function body(req) {
  if (req.headers['content-type']?.split(';')[0].trim().toLowerCase() !== 'application/json') throw fail('Expected application/json', 400);
  if (req.headers['content-length'] && (!/^\d+$/.test(req.headers['content-length']) || Number(req.headers['content-length']) > 1024)) throw fail('Request body too large', 400);
  const chunks = [];
  let size = 0;
  for await (const chunk of req) {
    size += chunk.length;
    if (size > 1024) throw fail('Request body too large', 400);
    chunks.push(chunk);
  }
  try { return JSON.parse(Buffer.concat(chunks).toString('utf8')); }
  catch { throw fail('Invalid JSON', 400); }
}
async function main() {
  assert(process.env.FAUCET_CONFIG_FILE, 'FAUCET_CONFIG_FILE required');
  const c = configFromFile(process.env.FAUCET_CONFIG_FILE);
  let faucet, chain, ready = false, stopping = false, timer;
  const active = new Set();
  const send = (res, status, value) => {
    if (res.destroyed) return;
    res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store', 'x-content-type-options': 'nosniff' });
    res.end(json(value));
  };
  const server = http.createServer((req, res) => {
    const work = (async () => {
      if (!ready || stopping) throw fail('Faucet not ready');
      if (active.size >= 32) throw fail('Faucet request capacity reached');
      if (req.url.length > 1024) throw fail('Request URL too long', 400);
      authorizeRequest(req, c.origin);
      const url = new URL(req.url, c.origin);
      if (url.origin !== c.origin) throw fail('Origin rejected', 403);
      const route = url.pathname;
      if (!['/faucet/config', '/faucet/request', '/faucet/status'].includes(route)) throw fail('Not found', 404);
      if (req.method !== (route === '/faucet/request' ? 'POST' : 'GET')) throw fail('Method not allowed', 405);
      if (route === '/faucet/config') return send(res, 200, { genesis: GENESIS, chainId: CHAIN_ID, assets: { TVARA: { amount: AMOUNTS.TVARA, decimals: 12 }, USDC: { amount: AMOUNTS.USDC, decimals: 6, contract: TOKEN } }, cooldownMs: COOLDOWN });
      if (route === '/faucet/status') {
        if ([...url.searchParams.keys()].some(k => !['asset', 'address'].includes(k)) || url.searchParams.getAll('asset').length !== 1 || url.searchParams.getAll('address').length !== 1) throw fail('Invalid status query', 400);
        return send(res, 200, faucet.status(url.searchParams.get('asset'), url.searchParams.get('address')));
      }
      if (req.headers.origin !== c.origin) throw fail('Exact Origin required', 403);
      const input = await bounded(body(req), 5000);
      if (!input || typeof input !== 'object' || Array.isArray(input) || Object.keys(input).sort().join(',') !== 'address,asset,chainId,genesis' || input.genesis !== GENESIS || input.chainId !== CHAIN_ID) throw fail('Invalid request or network', 400);
      return send(res, 202, await faucet.request(input.asset, input.address));
    })().catch(error => send(res, error.publicMessage && [400, 403, 404, 405, 429, 503].includes(error.code) ? error.code : 503, { error: error.publicMessage || 'Faucet held: network, funding or journal validation unavailable', ...(error.nextClaimAt ? { nextClaimAt: error.nextClaimAt } : {}) }));
    active.add(work);
    work.finally(() => active.delete(work));
  });
  server.requestTimeout = 10000;
  server.headersTimeout = 10000;
  server.timeout = 60000;
  server.keepAliveTimeout = 5000;
  server.maxHeadersCount = 32;
  server.maxConnections = 64;
  // Bind the fixed loopback ownership lock before keys, journals or RPC writes.
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(5485, '127.0.0.1', resolve); });
  async function stop() {
    if (stopping) return;
    stopping = true;
    ready = false;
    clearTimeout(timer);
    if (faucet) faucet.stopping = true;
    // Do not release the port until all possible writes/broadcasts have stopped.
    server.closeIdleConnections();
    const force = setTimeout(() => process.exit(1), 65000);
    await Promise.allSettled([...active, ...(faucet ? [...faucet.jobs.values()] : [])]);
    await chain?.close();
    await new Promise(resolve => server.close(resolve));
    clearTimeout(force);
  }
  process.once('SIGTERM', () => { stop().then(() => process.exit(0)); });
  process.once('SIGINT', () => { stop().then(() => process.exit(0)); });
  try {
    chain = await connect(c);
    await bounded(chain.identityCheck(), 30000);
    if (stopping) return;
    const journal = new Journal(c.stateDir, chain.secret, chain.identity);
    for (const claim of journal.claims) chain.validate(claim);
    faucet = new Faucet(journal, chain);
    ready = true;
    console.log('Isolated faucet ready on 127.0.0.1:5485; unresolved originals are observation-only');
    async function tick() {
      await Promise.allSettled(['TVARA', 'USDC'].map(asset => faucet.observe(asset)));
      if (!stopping) timer = setTimeout(tick, 4000);
    }
    timer = setTimeout(tick, 1000);
  } catch { console.error('Faucet startup held: configuration, chain identity or journal validation failed'); await stop(); process.exit(1); }
}
module.exports = { Journal, Faucet, admission, normalize, approvedOrigin, authorizeRequest, AMOUNTS, COOLDOWN, GENESIS, ETH_GENESIS, TOKEN, CODE_HASH, ORIGIN, CHAIN_ID, ABI };
if (require.main === module) main().catch(() => { console.error('Faucet could not acquire exclusive loopback ownership'); process.exit(1); });
