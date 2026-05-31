// Tellur JS API wrapper
const { execFileSync } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

function getBinary() {
  const name = os.platform() === 'win32' ? 'tellur.exe' : 'tellur';
  // Downloaded by install.js into ./bin
  const installed = path.resolve(__dirname, 'bin', name);
  if (fs.existsSync(installed)) return installed;
  // Locally built (monorepo dev)
  const local = path.resolve(__dirname, '..', '..', 'target', 'release', name);
  if (fs.existsSync(local)) return local;
  // Fall back to PATH
  return name;
}

function run(args) {
  const result = execFileSync(getBinary(), args, { encoding: 'utf8' });
  return result.trim();
}

module.exports = {
  /** Initialize Tellur in a repository */
  init() { return run(['init']); },

  /** Explain who changed a line */
  explain(file, line) {
    return JSON.parse(run(['explain', `${file}:${line}`, '--json']));
  },

  /** Show file attribution */
  blame(file) {
    return JSON.parse(run(['blame', file, '--json']));
  },

  /** Generate PR report */
  prReport(base = 'main', head = 'HEAD') {
    return run(['pr-report', '--base', base, '--head', head]);
  },

  /** List sessions */
  sessions() {
    return JSON.parse(run(['sessions', '--json']));
  },

  /** Check policy */
  policyCheck() {
    return run(['policy', 'check']);
  },

  /** Verify integrity */
  verify() {
    return run(['verify']);
  },
};
