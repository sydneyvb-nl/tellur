// Tellur post-install — download and verify the platform binary.
//
// Downloads the release archive for the current platform from GitHub Releases,
// verifies its SHA-256 against the published .sha256 sidecar, extracts the
// `tellur` binary into ./bin, and makes it executable.
//
// If anything fails (e.g. no release yet, offline install), it exits 0 with a
// hint to build from source rather than breaking `npm install`.

const https = require('https');
const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { execFileSync } = require('child_process');

const VERSION = require('./package.json').version;
const REPO = 'sydneyvb-nl/tellur';

const ARCHIVE_MAP = {
  'darwin-arm64': 'tellur-mac-arm64.tar.gz',
  'darwin-x64': 'tellur-mac-x64.tar.gz',
  'linux-x64': 'tellur-linux-x64.tar.gz',
  'linux-arm64': 'tellur-linux-arm64.tar.gz',
  'win32-x64': 'tellur-windows-x64.zip',
};

function bail(msg) {
  console.log(msg);
  console.log('Build from source instead: cargo install --git https://github.com/' + REPO + ' tellur-cli');
  process.exit(0);
}

function download(url) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { 'User-Agent': 'tellur-installer' } }, (res) => {
        if (res.statusCode === 302 || res.statusCode === 301) {
          return resolve(download(res.headers.location));
        }
        if (res.statusCode !== 200) {
          return reject(new Error('HTTP ' + res.statusCode + ' for ' + url));
        }
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => resolve(Buffer.concat(chunks)));
      })
      .on('error', reject);
  });
}

async function main() {
  const key = `${os.platform()}-${os.arch()}`;
  const archive = ARCHIVE_MAP[key];
  if (!archive) {
    bail(`No prebuilt Tellur binary for ${key}.`);
  }

  const base = `https://github.com/${REPO}/releases/download/v${VERSION}/${archive}`;
  console.log(`Tellur v${VERSION} — downloading ${archive}...`);

  let archiveBuf, expectedSha;
  try {
    archiveBuf = await download(base);
    const shaText = (await download(base + '.sha256')).toString('utf8').trim();
    // sha256 sidecar may be "<hash>  <file>" or just "<hash>"
    expectedSha = shaText.split(/\s+/)[0].toLowerCase();
  } catch (e) {
    bail(`Download failed (${e.message}). Release may not be published yet.`);
  }

  const actualSha = crypto.createHash('sha256').update(archiveBuf).digest('hex');
  if (expectedSha && actualSha !== expectedSha) {
    console.error('Tellur checksum verification FAILED.');
    console.error(`  expected: ${expectedSha}`);
    console.error(`  actual:   ${actualSha}`);
    process.exit(1); // hard fail — never install an unverified binary
  }
  console.log('Checksum verified.');

  const binDir = path.resolve(__dirname, 'bin');
  fs.mkdirSync(binDir, { recursive: true });
  const tmp = path.join(os.tmpdir(), `tellur-${Date.now()}-${archive}`);
  fs.writeFileSync(tmp, archiveBuf);

  // Extract the canonical `tellur`/`tellur.exe` into bin/.
  const isWin = os.platform() === 'win32';
  if (isWin) {
    execFileSync('tar', ['-xf', tmp, '-C', binDir], { stdio: 'inherit' }); // bsdtar on Win10+
  } else {
    execFileSync('tar', ['-xzf', tmp, '-C', binDir], { stdio: 'inherit' });
  }
  fs.unlinkSync(tmp);

  const binName = isWin ? 'tellur.exe' : 'tellur';
  const binPath = path.join(binDir, binName);
  if (!fs.existsSync(binPath)) {
    bail('Archive did not contain the expected tellur binary.');
  }
  if (!isWin) fs.chmodSync(binPath, 0o755);
  console.log(`Tellur installed at ${binPath}`);
}

main().catch((e) => bail(`Install error: ${e.message}`));
