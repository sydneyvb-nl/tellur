# Contributing to TraceGit

Thanks for your interest in contributing! TraceGit is an open-source AI code provenance platform and we welcome contributions of all kinds.

## Development Setup

```bash
# Clone the repo
git clone https://github.com/sydneyvb-nl/TraceGit.git
cd TraceGit

# Install dependencies
npm install

# Build all packages
npm run build

# Run tests
npm run test
```

## Project Structure

```
TraceGit/
├── packages/
│   ├── core/          # Schemas, attribution engine, storage, policy
│   ├── cli/           # CLI interface (tracegit command)
│   ├── adapters/      # AI tool adapters
│   └── vscode/        # VS Code extension (planned)
├── schemas/           # JSON Schema definitions
└── docs/              # Documentation
```

## Code Style

- TypeScript strict mode — no `any`, no `// @ts-ignore`
- Node16 module resolution
- ESM with `.js` extensions in imports
- 2-space indentation
- Descriptive variable names, no abbreviations

## Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add Claude Code adapter hook installer
fix: handle missing git root in init command
docs: add adapter authoring guide
test: add event schema validation tests
chore: update dependencies
```

## Pull Requests

1. Create a feature branch from `main`
2. Make your changes with tests
3. Ensure `npm run build` and `npm run test` pass
4. Submit a PR with a clear description

## Reporting Issues

- Use GitHub Issues
- Include steps to reproduce
- Include TraceGit version (`tracegit --version`)
- Include Node.js version (`node --version`)

## License

By contributing, you agree that your contributions will be licensed under the Apache-2.0 license.
