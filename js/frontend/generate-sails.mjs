import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

const frontend = dirname(fileURLToPath(import.meta.url));
const root = resolve(frontend, '../..');
const sdkRequire = createRequire(new URL('../bridge-js/package.json', import.meta.url));
const cliPackagePath = sdkRequire.resolve('sails-js-cli/package.json');
const cliPackage = JSON.parse(readFileSync(cliPackagePath, 'utf8'));
if (cliPackage.version !== '0.5.0') throw new Error('Client generation requires sails-js-cli 0.5.0');
const cli = resolve(dirname(cliPackagePath), cliPackage.bin['sails-js']);
const args = process.argv.slice(2);
if (args.length > 1 || (args.length === 1 && args[0] !== '--check')) {
  throw new Error('Usage: node js/frontend/generate-sails.mjs [--check]');
}
const check = args[0] === '--check';
const clients = [
  ['vft', 'src/consts/sails/vft.ts'],
  ['vft_vara', 'src/consts/sails/vft-vara.ts'],
  ['bridging_payment', 'src/features/swap/consts/sails/bridging-payment.ts'],
  ['checkpoint_light_client', 'src/features/swap/consts/sails/checkpoint-client.ts'],
  ['eth_events_electra', 'src/features/swap/consts/sails/eth-events.ts'],
  ['historical_proxy', 'src/features/swap/consts/sails/historical-proxy.ts'],
  ['vft_manager', 'src/features/swap/consts/sails/vft-manager.ts'],
];

function parse(name, text) {
  const source = ts.createSourceFile(name, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  if (source.parseDiagnostics.length) throw new Error(`Invalid generated TypeScript: ${name}`);
  return source;
}

function collect(node, predicate) {
  const found = [];
  function visit(child) {
    if (predicate(child)) found.push(child);
    ts.forEachChild(child, visit);
  }
  visit(node);
  return found;
}

function exactlyOne(nodes, label) {
  if (nodes.length !== 1) throw new Error(`Unexpected generator shape: ${label} (${nodes.length})`);
  return nodes[0];
}

function adapt(lib, declarations, idlSha256) {
  const source = parse('lib.ts', lib);
  const types = parse('global.d.ts', declarations);
  const imports = new Map();
  const edits = [];
  const inlineTypes = [];
  function addImport(statement) {
    const clause = statement.importClause;
    if (!clause || clause.name || clause.isTypeOnly || !ts.isNamedImports(clause.namedBindings)) {
      throw new Error('Unexpected generated import shape');
    }
    const module = statement.moduleSpecifier.text;
    const names = imports.get(module) ?? new Set();
    clause.namedBindings.elements.forEach((element) => names.add(element.getText()));
    imports.set(module, names);
  }
  for (const statement of source.statements) {
    if (ts.isImportDeclaration(statement)) {
      addImport(statement);
      edits.push({ start: statement.getStart(source), end: statement.end, text: '' });
    }
  }
  for (const statement of types.statements) {
    if (ts.isImportDeclaration(statement)) addImport(statement);
    else if (ts.isModuleDeclaration(statement) && statement.name.text === 'global' && ts.isModuleBlock(statement.body)) {
      inlineTypes.push(declarations.slice(statement.body.getStart(types) + 1, statement.body.end - 1).trim());
    } else if (!ts.isEmptyStatement(statement)) throw new Error('Unexpected generated type declaration');
  }

  // Keep the frontend's inline exported types and strict-property-initialization adaptation.
  for (const property of collect(source, (node) => ts.isPropertyDeclaration(node) && node.name.getText(source) === '_program')) {
    if (!property.exclamationToken && !property.questionToken && !property.initializer) {
      edits.push({ start: property.name.end, end: property.name.end, text: '!' });
    }
  }

  // sails-js-cli 0.5.0 emits undefined; sails-js 0.5.1 requires null for constructor services.
  for (const builder of collect(source, (node) => ts.isNewExpression(node) && node.expression.getText(source) === 'TransactionBuilder')) {
    const [,, extrinsic, service] = builder.arguments ?? [];
    if (!extrinsic || !ts.isStringLiteral(extrinsic) || !['upload_program', 'create_program'].includes(extrinsic.text)) continue;
    if (!service || !ts.isIdentifier(service) || service.text !== 'undefined') throw new Error('Unexpected generated constructor service');
    edits.push({ start: service.getStart(source), end: service.end, text: 'null' });
  }

  const subscriptions = collect(source, (node) => ts.isMethodDeclaration(node) && node.name.getText(source).startsWith('subscribeTo'));
  const registrations = collect(source, (node) => ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === 'subscribeToGearEvent');
  if (registrations.length !== subscriptions.length) throw new Error('Unadapted generated event registration');
  for (const method of subscriptions) {
    const registration = exactlyOne(collect(method, (node) => registrations.includes(node)), 'event registration');
    const listener = registration.arguments[1];
    if (registration.arguments[0]?.getText(source) !== "'UserMessageSent'" || !ts.isArrowFunction(listener) || !ts.isBlock(listener.body)) {
      throw new Error('Unexpected generated event listener');
    }
    const callback = exactlyOne(collect(listener.body, (node) => ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'callback'), 'event callback');
    if (callback.arguments.length !== 1) throw new Error('Unexpected generated callback arguments');
    const argument = callback.arguments[0];
    const decoders = collect(argument, (node) => ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === 'createType');
    let decode;
    let value;
    if (argument.kind === ts.SyntaxKind.NullKeyword) {
      decode = "this._program.registry.createType('(String, String)', message.payload)";
      value = 'null';
    } else {
      const decoder = exactlyOne(decoders, 'event decoder');
      decode = decoder.getText(source);
      value = lib.slice(argument.getStart(source), decoder.getStart(source)) + 'decoded' + lib.slice(decoder.end, argument.end);
    }
    // Decode and callback execution both stay inside subscribeAdvisory's error boundary.
    // Re-encoding validates complete canonical SCALE consumption, including zero-field events.
    const body = lib.slice(listener.body.getStart(source) + 1, callback.getStart(source))
      + `const decoded = ${decode};\n`
      + `        if (decoded.toHex() !== message.payload.toHex().toLowerCase()) throw new Error('Malformed or trailing advisory event payload');\n`
      + `        return ${value}`
      + lib.slice(callback.end, listener.body.end - 1);
    edits.push({
      start: method.body.getStart(source),
      end: method.body.end,
      text: `{\n    return subscribeAdvisory((dispatch) => this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {\n      dispatch(() => {${body}\n      });\n    }), callback);\n  }`,
    });
  }
  if (subscriptions.length) imports.set('@gear-js/bridge', new Set(['subscribeAdvisory']));
  for (const edit of edits.sort((a, b) => b.start - a.start)) {
    lib = lib.slice(0, edit.start) + edit.text + lib.slice(edit.end);
  }
  const output = [
    '/* eslint-disable */',
    '// Generated by generate-sails.mjs from api/gear; do not edit by hand.',
    ...[...imports].map(([module, names]) => `import { ${[...names].join(', ')} } from '${module}';`),
    '', `export const IDL_SHA256 = '${idlSha256}';`,
    '', lib.trim(), ...inlineTypes.map((text) => `\n${text}`), '',
  ].join('\n');
  parse('adapted.ts', output);
  return output;
}

const scratch = mkdtempSync(join(tmpdir(), 'gear-bridges-frontend-sails-'));
try {
  // Finish every generation/adaptation before modifying any checked-in client.
  const generated = clients.map(([idl, destination]) => {
    const out = join(scratch, idl);
    const input = join(root, 'api/gear', `${idl}.idl`);
    const idlSha256 = createHash('sha256').update(readFileSync(input)).digest('hex');
    execFileSync(process.execPath, [cli, 'generate', input, '--no-project', '--out', out, '--yes'], { stdio: 'inherit' });
    const types = join(out, 'global.d.ts');
    return [destination, adapt(readFileSync(join(out, 'lib.ts'), 'utf8'), existsSync(types) ? readFileSync(types, 'utf8') : '', idlSha256)];
  });
  const stale = [];
  for (const [destination, output] of generated) {
    const path = join(frontend, destination);
    if (check) {
      if (!existsSync(path) || readFileSync(path, 'utf8') !== output) stale.push(destination);
    } else writeFileSync(path, output);
  }
  if (stale.length) throw new Error(`Stale generated frontend clients:\n${stale.join('\n')}\nRun yarn workspace frontend generate:sails after exporting canonical Cargo IDLs.`);
  console.log(`${check ? 'Checked' : 'Generated'} ${generated.length} canonical frontend Sails clients`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
