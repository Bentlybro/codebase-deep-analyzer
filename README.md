# Codebase Deep Analyzer (CDA)

Fast codebase documentation for LLMs. Analyzes entire codebases in seconds and outputs structured docs optimized for AI consumption.

## Features

- **Fast** — ~15 seconds for 2500+ files
- **Single file output** — One `CODEBASE.md` that fits in LLM context
- **Multi-language** — Rust, TypeScript, JavaScript parsing via tree-sitter
- **Smart extraction** — Exports, imports, signatures, doc comments
- **Architecture overview** — LLM-generated summary of the codebase
- **JSON export** — Searchable structured data for programmatic use

## Installation

### From Release

```bash
# Linux
curl -L https://github.com/Bentlybro/codebase-deep-analyzer/releases/latest/download/cda-linux-x64 -o cda
chmod +x cda

# macOS
curl -L https://github.com/Bentlybro/codebase-deep-analyzer/releases/latest/download/cda-macos-x64 -o cda
chmod +x cda
```

### From Source

```bash
cargo install --path .
```

## Quick Start

```bash
# Set your API key (for architecture overview)
export ANTHROPIC_API_KEY=sk-...

# Analyze any codebase
cda analyze ./my-project -o ./docs

# Check the output
cat ./docs/CODEBASE.md
```

## Output

### Markdown (default)

Single `CODEBASE.md` containing:
- Architecture overview (LLM-generated)
- Directory structure with export counts
- All exports organized by directory
- External dependencies
- Internal dependency graph
- Documentation gaps

### JSON

```bash
cda analyze ./my-project -o ./docs -f json
```

Outputs `analysis.json` with structured data:
- Full module list with exports/imports
- Searchable export index
- Dependency mappings
- Cross-reference data

## Usage

```bash
# Standard analysis (fast, recommended)
cda analyze ./my-project -o ./docs

# JSON output for programmatic use
cda analyze ./my-project -o ./docs -f json

# Deep per-file LLM analysis (slow, use for small codebases only)
cda analyze ./my-project -o ./docs --deep -p 8
```

### Options

| Flag | Description |
|------|-------------|
| `-o, --output` | Output directory (default: ./cda-output) |
| `-f, --format` | Output format: markdown, json |
| `-m, --module` | Analyze specific module/directory |
| `--deep` | Enable slow per-file LLM analysis |
| `-p, --parallelism` | Workers for --deep mode (default: 4) |
| `-v, --verbose` | Verbose logging |

### LLM Providers

```bash
# Anthropic (default)
export ANTHROPIC_API_KEY=sk-...

# OpenAI
export OPENAI_API_KEY=sk-...
cda analyze ./project --provider openai

# Ollama (local)
cda analyze ./project --provider ollama
```

## How It Works

1. **Discovery** — Walks codebase respecting `.gitignore`
2. **Parsing** — Tree-sitter extracts exports, imports, signatures, doc comments
3. **Architecture** — One LLM call generates high-level overview
4. **Output** — Structured docs optimized for LLM consumption

## Example Output

```markdown
# Codebase Documentation

## Architecture
This TypeScript application is a multi-channel AI agent platform...

## Directory Structure
- `/src/agents` — 109 files, 466 exports
- `/src/channels` — 17 files, 75 exports
...

## Module Reference

### `/src/agents`

#### session.ts
- `createSession(config: SessionConfig): Session`
- `destroySession(id: string): void`
...
```

## License

MIT
