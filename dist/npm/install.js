// TraceGit post-install — download the correct binary
const https = require('https');
const fs = require('fs');
const path = require('path');
const os = require('os');

const VERSION = '0.1.0';
const REPO = 'sydneyvb-nl/TraceGit';

function getDownloadInfo() {
  const platform = os.platform();
  const arch = os.arch();

  const map = {
    'darwin-arm64': 'tracegit-mac-arm64.tar.gz',
    'darwin-x64': 'tracegit-mac-x64.tar.gz',
    'linux-x64': 'tracegit-linux-x64.tar.gz',
    'linux-arm64': 'tracegit-linux-arm64.tar.gz',
    'win32-x64': 'tracegit-windows-x64.zip',
  };

  const key = `${platform}-${arch}`;
  const filename = map[key];

  if (!filename) {
    console.log(`No prebuilt binary for ${key}. Build from source: cargo install --git https://github.com/${REPO}`);
    return null;
  }

  return {
    url: `https://github.com/${REPO}/releases/download/v${VERSION}/${filename}`,
    filename,
  };
}

const info = getDownloadInfo();
if (!info) process.exit(0);

console.log(`TraceGit v${VERSION} — downloading binary...`);

// For now, just create bin directory. Actual download happens on first release.
const binDir = path.resolve(__dirname, 'bin');
if (!fs.existsSync(binDir)) {
  fs.mkdirSync(binDir, { recursive: true });
}

console.log('Binary will be available after first GitHub release.');
console.log('For now, build locally: cargo build --release');
