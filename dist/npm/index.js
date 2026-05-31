// TraceGit JS API wrapper
const { execFileSync } = require('child_process');
const path = require('path');

function getBinary() {
  return path.resolve(__dirname, '..', '..', 'target', 'release', 'tracegit');
}

function run(args) {
  const result = execFileSync(getBinary(), args, { encoding: 'utf8' });
  return result.trim();
}

module.exports = {
  /** Initialize TraceGit in a repository */
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
