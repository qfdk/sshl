// Bench: 用 ssh2 直接连接，对照 OpenSSH 的耗时
const {Client} = require('ssh2');
const fs = require('fs');
const path = require('path');
const os = require('os');
const net = require('net');

const host = process.argv[2] || 'fro2.qfdk.me';
const user = process.argv[3] || 'root';
const useNodelay = process.argv.includes('--nodelay');
const keyPath = path.join(os.homedir(), '.ssh', 'id_rsa');

const t0 = Date.now();
const conn = new Client();
let tReady, tShell, tFirstData;

const baseOpts = {
    host, port: 22, username: user,
    privateKey: fs.readFileSync(keyPath),
    readyTimeout: 30000,
};

if (useNodelay) {
    const sock = net.connect({host, port: 22});
    sock.setNoDelay(true);
    baseOpts.sock = sock;
    console.log('[bench] using custom socket with TCP_NODELAY');
}

conn.on('ready', () => {
    tReady = Date.now();
    console.log(`[bench] ready ${tReady - t0}ms`);
    conn.shell({term: 'xterm-color', rows: 24, cols: 80}, (err, stream) => {
        if (err) { console.error(err); process.exit(1); }
        tShell = Date.now();
        console.log(`[bench] shell callback ${tShell - tReady}ms (since ready)`);
        stream.once('data', (d) => {
            tFirstData = Date.now();
            console.log(`[bench] first data ${tFirstData - tShell}ms (since shell)`);
            console.log(`[bench] TOTAL ${tFirstData - t0}ms`);
            console.log('[bench] first bytes:', JSON.stringify(d.toString().slice(0, 80)));
            stream.end('exit\n');
        });
        stream.on('close', () => conn.end());
    });
}).on('error', (err) => {
    console.error('[bench] error:', err.message);
    process.exit(1);
});

conn.connect(baseOpts);
