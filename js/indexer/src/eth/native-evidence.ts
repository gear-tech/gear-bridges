import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { TextDecoder } from 'node:util';

export class EthereumEvidenceError extends Error {
  constructor(readonly kind: 'unavailable' | 'conflict' | 'protocol', message: string) {
    super(`HOLD: ${message}`);
    this.name = 'EthereumEvidenceError';
  }
  get retryable(): boolean { return this.kind === 'unavailable'; }
}

export function exactObject(value: unknown, fields: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value) ||
      Object.keys(value).length !== fields.length || fields.some((field) => !Object.hasOwn(value, field))) {
    throw new EthereumEvidenceError('protocol', 'malformed native evidence object');
  }
  return value as Record<string, unknown>;
}

// JSON.parse accepts duplicate keys; protocol evidence must not have two interpretations.
function parseFrame(text: string): Record<string, unknown> {
  const value: unknown = JSON.parse(text);
  const stack: Array<{ keys?: Set<string>; key: boolean }> = [];
  const tokens = text.match(/"(?:[^"\\]|\\.)*"|[{}[\]:,]|[^\s{}[\]:,]+/g) ?? [];
  for (const token of tokens) {
    const current = stack.at(-1);
    if (token === '{') stack.push({ keys: new Set(), key: true });
    else if (token === '[') stack.push({ key: false });
    else if (token === '}' || token === ']') stack.pop();
    else if (token === ',') { if (current?.keys) current.key = true; }
    else if (token === ':') { if (current) current.key = false; }
    else if (current?.keys && current.key) {
      const key = JSON.parse(token) as string;
      if (current.keys.has(key)) throw new Error('duplicate key');
      current.keys.add(key);
      current.key = false;
    }
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('invalid frame');
  return value as Record<string, unknown>;
}

export class NativeEvidenceClient {
  private readonly child: ChildProcessWithoutNullStreams;
  private queued: Promise<unknown> = Promise.resolve();
  private failure?: EthereumEvidenceError;
  private pending?: { id: string; op: string; resolve(value: Record<string, unknown>): void; reject(error: Error): void };
  private nextId = 0n;
  private buffer = Buffer.alloc(0);
  private readonly onExit = () => this.close();

  constructor(binary: string, args: string[]) {
    this.child = spawn(binary, args, { shell: false, stdio: ['pipe', 'pipe', 'pipe'] });
    this.child.stderr.on('data', (chunk: Buffer) => process.stderr.write(chunk));
    this.child.stdout.on('data', (chunk: Buffer) => this.receive(chunk));
    this.child.stdout.on('error', () => this.fail('unavailable', 'native evidence output closed'));
    this.child.stdout.on('end', () => this.fail(this.buffer.length ? 'protocol' : 'unavailable',
      this.buffer.length ? 'truncated native evidence frame' : 'native evidence output ended'));
    this.child.on('error', () => this.fail('unavailable', 'native evidence child unavailable'));
    this.child.stdin.on('error', () => this.fail('unavailable', 'native evidence input closed'));
    this.child.on('close', () => this.fail('unavailable', 'native evidence child exited'));
    process.once('exit', this.onExit);
  }

  request(input: Record<string, unknown>): Promise<Record<string, unknown>> {
    const result = this.queued.then(() => {
      if (this.failure) throw this.failure;
      const id = (++this.nextId).toString();
      const frame = JSON.stringify({ ...input, id }) + '\n';
      if (Buffer.byteLength(frame) > 8 * 1024 * 1024) {
        throw new EthereumEvidenceError('protocol', 'native evidence request exceeds frame limit');
      }
      return new Promise<Record<string, unknown>>((resolve, reject) => {
        this.pending = { id, op: String(input.op), resolve, reject };
        this.child.stdin.write(frame);
      });
    });
    this.queued = result.catch(() => undefined);
    return result.then((response) => {
      if (this.failure) throw this.failure;
      return response;
    });
  }

  close(): void { this.fail('unavailable', 'native evidence owner closed'); }

  private fail(kind: EthereumEvidenceError['kind'], message: string): void {
    if (this.failure) return;
    this.failure = new EthereumEvidenceError(kind, message);
    this.pending?.reject(this.failure);
    this.pending = undefined;
    this.buffer = Buffer.alloc(0);
    process.removeListener('exit', this.onExit);
    this.child.stdin.destroy();
    this.child.stdout.destroy();
    this.child.stderr.destroy();
    this.child.kill('SIGTERM');
  }

  private receive(chunk: Buffer): void {
    if (this.failure) return;
    if (this.buffer.length + chunk.length > 8 * 1024 * 1024) {
      this.fail('protocol', 'native evidence response exceeds frame limit');
      return;
    }
    this.buffer = Buffer.concat([this.buffer, chunk]);
    let end: number;
    while ((end = this.buffer.indexOf(10)) !== -1) {
      const line = this.buffer.subarray(0, end);
      this.buffer = this.buffer.subarray(end + 1);
      try {
        const message = parseFrame(new TextDecoder('utf-8', { fatal: true }).decode(line));
        const pending = this.pending;
        if (!pending || message.id !== pending.id || typeof message.ok !== 'boolean') throw new Error('unmatched reply');
        if (!message.ok) {
          exactObject(message, ['id', 'ok', 'error']);
          const error = exactObject(message.error, ['kind', 'message']);
          if (!['unavailable', 'conflict', 'protocol'].includes(String(error.kind)) || typeof error.message !== 'string') {
            throw new Error('invalid error');
          }
          this.pending = undefined;
          // The child's diagnostics are deliberately not reflected: serving endpoints may contain credentials.
          pending.reject(new EthereumEvidenceError(error.kind as EthereumEvidenceError['kind'], `native evidence ${error.kind}`));
          if (error.kind === 'protocol') this.fail('protocol', 'native evidence protocol rejected');
        } else {
          if (message.op !== pending.op) throw new Error('wrong operation');
          exactObject(message, pending.op === 'qualify'
            ? ['id', 'ok', 'op', 'checkpoint', 'headers', 'requalificationLinks']
            : ['id', 'ok', 'op', 'checkpoint', 'receipts']);
          this.pending = undefined;
          pending.resolve(message);
        }
      } catch {
        this.fail('protocol', 'malformed or unmatched native evidence reply');
        return;
      }
    }
  }
}
