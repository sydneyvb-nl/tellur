#!/usr/bin/env node
// Tellur CLI wrapper — downloads and runs the native binary

const { execFileSync } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

function getBinaryPath() {
  const platform = os.platform();
  const arch = os.arch();

  let name = 'tellur';
  if (platform === 'win32') name += '.exe';

  // Check for locally built binary first
  const localPath = path.resolve(__dirname, '..', '..', 'target', 'release', name);
  if (fs.existsSync(localPath)) return localPath;

  // Check for installed binary
  const installedPath = path.resolve(__dirname, 'bin', name);
  if (fs.existsSync(installedPath)) return installedPath;

  // Fallback: assume it's on PATH
  return name;
}

const binary = getBinaryPath();
const args = process.argv.slice(2);

try {
  const result = execFileSync(binary, args, {
    stdio: 'inherit',
    env: process.env,
  });
  process.exit(result.status || 0);
} catch (e) {
  if (e.status) {
    process.exit(e.status);
  }
  console.error('Tellur binary not found. Install with: tellur init');
  process.exit(1);
}
