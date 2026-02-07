# Codebase Deep Analyzer (CDA)

A systematic codebase exploration tool that thoroughly documents and understands any codebase using LLM-assisted analysis.

## Problem

When exploring a codebase, it's easy to do surface-level analysis and miss existing functionality. CDA solves this by:

1. **Exhaustive discovery** — Catalogs all files by type (source, config, docs, tests)
2. **Deep module analysis** — Uses LLM to understand each module's purpose and exports
3. **Cross-referencing** — Maps dependencies and finds gaps
4. **Verification** — Actually runs commands to confirm documented behavior exists

## Installation

### From Release

```bash
# Download latest release
curl -L https://github.com/Bentlybro/codebase-deep-analyzer/releases/latest/download/cda-linux-x64 -o cda
chmod +x cda
./cda --help
```

### From Source

```bash
cargo install --path .
```

## Usage

### Analyze a Codebase

```bash
# Full analysis with LLM assistance
cda analyze ./my-project --output ./docs

# Static analysis only (no LLM, faster)
cda analyze ./my-project --static-only

# Analyze specific module
cda analyze ./my-project --module src/core

# JSON output for programmatic use
cda analyze ./my-project --format json
```

### Configuration

```bash
# Create config file
cda config --init

# View current config
cda config
```

Or use environment variables:

```bash
export ANTHROPIC_API_KEY=sk-...
export CDA_PROVIDER=anthropic
export CDA_MODEL=claude-sonnet-4-20250514
```

### LLM Providers

- **anthropic** — Claude (default: claude-sonnet-4-20250514)
- **openai** — GPT-4 (default: gpt-4o)
- **ollama** — Local models (default: llama3)

## Output

CDA generates structured documentation:

```
cda-output/
├── index.md          # Overview with links to all modules
├── modules/
│   ├── core.md       # Each module documented
│   ├── cli.md
│   └── ...
├── gaps.md           # Potential issues found
└── analysis.json     # Machine-readable full analysis
```

## How It Works

### Phase 1: Discovery
- Walk the codebase respecting `.gitignore`
- Categorize files: source, config, docs, tests
- Identify entry points and exports

### Phase 2: Module Analysis
- Parse each source file with tree-sitter
- Use LLM to understand purpose and behavior
- Extract all exports with signatures

### Phase 3: Cross-Reference
- Map module dependencies
- Find unused exports (dead code)
- Identify untested functions
- Flag undocumented commands

### Phase 4: Output
- Generate structured markdown docs
- Create searchable JSON export
- List gaps and recommendations

## License

MIT
