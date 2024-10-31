const fs = require('fs');
const path = require('path');

const abiPath = process.argv[2];

const abi = JSON.parse(fs.readFileSync(abiPath, 'utf8'));

fs.writeFileSync(path.join('./assets', abiPath.split('/').at(-1)), JSON.stringify(abi.abi, null, 2));
