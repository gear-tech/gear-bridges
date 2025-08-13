const fs = require('fs');
const path = require('path');

const [abiPath, destPath] = process.argv.slice(2);

const abi = JSON.parse(fs.readFileSync(abiPath, 'utf8'));

fs.writeFileSync(path.join(destPath, abiPath.split('/').at(-1)), JSON.stringify(abi.abi, null, 2));
