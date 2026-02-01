# EVI Portability: Applying to Other Projects

**Version:** 0.1.0
**Last Updated:** 2026-01-22
**Status:** Design Document

---

## Executive Summary

EVI is designed to be **project-agnostic infrastructure**. While initially developed for Kelpie, the components can be packaged and applied to any software project. This document describes:

1. What's project-specific vs. reusable
2. How to package EVI for distribution
3. How to apply EVI to a new project
4. Configuration and customization points

---

## Table of Contents

1. [Component Portability Analysis](#1-component-portability-analysis)
2. [Packaging Strategy](#2-packaging-strategy)
3. [Installation Guide](#3-installation-guide)
4. [Project Adaptation](#4-project-adaptation)
5. [Distribution Model](#5-distribution-model)

---

## 1. Component Portability Analysis

### 1.1 Fully Reusable Components

These components are completely project-agnostic and can be used as-is:

| Component | Location | Portability | Notes |
|-----------|----------|-------------|-------|
| **MCP Server Core** | `kelpie-mcp/mcp_kelpie/` | ✅ 100% | Only needs `CODEBASE_PATH` config |
| **REPL Tools** | `mcp_kelpie/rlm/` | ✅ 100% | Language-agnostic RLM execution |
| **AgentFS** | `mcp_kelpie/agentfs/` | ✅ 100% | Generic persistence layer |
| **Examination System** | `mcp_kelpie/tools/handlers.py` (exam_*) | ✅ 100% | Generic completeness gates |
| **Python Indexer** | `mcp_kelpie/indexer/` | ⚠️ 90% | Supports Rust only (needs language adapters) |

### 1.2 Template Components (Require Adaptation)

These components provide templates that need project-specific customization:

| Component | Location | Customization Needed |
|-----------|----------|---------------------|
| **Skills** | `.claude/skills/` | Update for project structure |
| **CLAUDE.md** | Project root | Update architecture, conventions |
| **Hooks** | `hooks/` | Update for project's build/test commands |
| **.vision/** | `.vision/` | Project-specific constraints |

### 1.3 Project-Specific Components

These are Kelpie-specific and not portable:

| Component | Location | Why Not Portable |
|-----------|----------|------------------|
| **DST Framework** | `crates/kelpie-dst/` | Kelpie's testing infrastructure |
| **Structural Indexes** | `.kelpie-index/` | Generated, not distributed |
| **AgentFS Data** | `.agentfs/` | Session data, not portable |

---

## 2. Packaging Strategy

### 2.1 Proposed Package Structure

```
evi/
├── pyproject.toml                  # Python package metadata
├── README.md                       # Installation & usage
├── LICENSE                         # Open source license
│
├── evi/                            # Renamed from mcp_kelpie
│   ├── __init__.py
│   ├── server.py                   # MCP server entry point
│   │
│   ├── rlm/                        # RLM execution environment
│   │   ├── repl.py
│   │   ├── llm.py
│   │   ├── context.py
│   │   └── types.py
│   │
│   ├── agentfs/                    # Persistence layer
│   │   ├── session.py
│   │   └── wrapper.py
│   │
│   ├── indexer/                    # Code indexing
│   │   ├── indexer.py
│   │   ├── parser.py
│   │   ├── types.py
│   │   └── languages/              # Language-specific parsers
│   │       ├── rust.py
│   │       ├── python.py           # Future
│   │       ├── typescript.py       # Future
│   │       └── go.py               # Future
│   │
│   └── tools/                      # MCP tool handlers
│       ├── definitions.py
│       └── handlers.py
│
├── templates/                      # Templates for new projects
│   ├── .mcp.json
│   ├── .env.example
│   ├── CLAUDE.md
│   ├── .vision/
│   │   ├── CONSTRAINTS.md
│   │   └── EVI.md
│   └── skills/
│       ├── codebase-map/
│       └── thorough-answer/
│
├── docs/
│   ├── installation.md
│   ├── quickstart.md
│   ├── customization.md
│   └── architecture.md
│
└── tests/                          # Tests for EVI itself
    ├── test_rlm.py
    ├── test_indexer.py
    ├── test_agentfs.py
    └── test_tools.py
```

### 2.2 Package Metadata

**pyproject.toml:**
```toml
[project]
name = "evi"
version = "0.1.0"
description = "Exploration & Verification Infrastructure for AI agent-driven development"
authors = [{name = "Your Name", email = "you@example.com"}]
readme = "README.md"
requires-python = ">=3.11"
license = {text = "MIT"}

dependencies = [
    "anthropic>=0.40.0",
    "agentfs-sdk>=0.1.0",
    "tree-sitter>=0.20.0",
    "tree-sitter-rust>=0.20.0",
    "mcp>=1.0.0",
]

[project.optional-dependencies]
dev = [
    "pytest>=7.0.0",
    "pytest-asyncio>=0.21.0",
    "black>=23.0.0",
    "ruff>=0.1.0",
]

[project.scripts]
evi = "evi.server:main"

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"
```

### 2.3 Distribution Channels

| Channel | Purpose | Audience |
|---------|---------|----------|
| **PyPI** | `pip install evi` | General users |
| **GitHub** | Source code, issues, PRs | Contributors |
| **Docker Hub** | `docker pull evi/mcp-server` | Container users |
| **NPM** | `npx create-evi-project` | Quick setup |

---

## 3. Installation Guide

### 3.1 Quick Install

```bash
# Install EVI
pip install evi

# Initialize in your project
cd /path/to/your/project
evi init

# This creates:
# - .mcp.json (MCP server config)
# - .env.example (environment template)
# - CLAUDE.md (from template, needs customization)
# - .vision/ (vision files from template)
# - .claude/skills/ (skills from template)
```

### 3.2 What `evi init` Does

```
┌─────────────────────────────────────────────────────────────────┐
│  evi init --language=rust --test-framework=dst                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. Detect project structure                                   │
│     ✓ Found Cargo.toml (Rust project)                          │
│     ✓ Found crates/ directory                                  │
│     ✓ Detected test framework: cargo test                      │
│                                                                 │
│  2. Copy template files                                        │
│     ✓ .mcp.json → configured with project path                 │
│     ✓ .env.example → with ANTHROPIC_API_KEY                    │
│     ✓ CLAUDE.md → Rust-specific template                       │
│     ✓ .vision/CONSTRAINTS.md → template                        │
│     ✓ .vision/EVI.md → generic EVI vision                      │
│     ✓ .claude/skills/ → codebase-map, thorough-answer          │
│                                                                 │
│  3. Build initial indexes                                      │
│     ✓ Scanning codebase...                                     │
│     ✓ Built symbols.json (1,234 symbols)                       │
│     ✓ Built modules.json (56 modules)                          │
│     ✓ Built dependencies.json (12 crates)                      │
│     ✓ Built tests.json (234 tests)                             │
│                                                                 │
│  4. Next steps                                                 │
│     1. Copy .env.example to .env and add your ANTHROPIC_API_KEY│
│     2. Customize CLAUDE.md with your project conventions       │
│     3. Update .vision/CONSTRAINTS.md with your requirements    │
│     4. Run: evi serve (starts MCP server)                      │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 3.3 Manual Installation

For projects that need more control:

```bash
# 1. Install package
pip install evi

# 2. Create config directory
mkdir -p .evi

# 3. Generate config interactively
evi config

# Questions:
# - Project language? [rust/python/typescript/go]
# - Test framework? [cargo test/pytest/jest/go test]
# - Verification framework? [DST/property tests/none]
# - Index path? [.evi-index]
# - Sub-LLM model? [claude-haiku-4-5-20251001]

# 4. Manually copy and customize templates
cp -r $(evi template-path)/CLAUDE.md ./
cp -r $(evi template-path)/.vision ./
cp -r $(evi template-path)/skills ./.claude/

# 5. Build indexes
evi index build

# 6. Start server
evi serve
```

---

## 4. Project Adaptation

### 4.1 Language-Specific Indexers

EVI needs to understand your project's language. Current support:

| Language | Status | Parser | Symbol Types |
|----------|--------|--------|--------------|
| **Rust** | ✅ Full | tree-sitter-rust | struct, enum, trait, fn, impl |
| **Python** | ⚠️ Planned | tree-sitter-python | class, def, async def |
| **TypeScript** | ⚠️ Planned | tree-sitter-typescript | interface, class, function |
| **Go** | ⚠️ Planned | tree-sitter-go | struct, interface, func |

**Adding Language Support:**

```python
# evi/indexer/languages/python.py
from tree_sitter import Language, Parser
from ..types import Symbol, Module

class PythonIndexer:
    def __init__(self):
        self.parser = Parser()
        self.parser.set_language(Language('python'))

    def extract_symbols(self, source: str, file_path: str) -> list[Symbol]:
        tree = self.parser.parse(bytes(source, "utf8"))
        symbols = []

        for node in tree.root_node.children:
            if node.type == "class_definition":
                symbols.append(Symbol(
                    name=self._get_name(node),
                    kind="class",
                    file_path=file_path,
                    line=node.start_point[0] + 1,
                ))
            elif node.type == "function_definition":
                symbols.append(Symbol(
                    name=self._get_name(node),
                    kind="function",
                    file_path=file_path,
                    line=node.start_point[0] + 1,
                ))

        return symbols

    # ... rest of implementation
```

### 4.2 Customizing Skills

Skills need to be adapted for your project structure:

**Example: Python Django Project**

```markdown
# .claude/skills/codebase-map/SKILL.md

## When to Use
- User asks to "map the codebase"
- User needs to understand Django app structure

## Workflow

### Step 1: Initialize
exam_start(task="Map Django project", scope=["all"])

### Step 2: Examine Each Django App
For EACH app in the project:

#### 2a. Get Structure (use indexes)
index_modules(pattern="myapp/**/*.py")
index_symbols(kind="class", pattern="myapp/models.py")  # Django models
index_symbols(kind="class", pattern="myapp/views.py")   # Views
index_symbols(kind="function", pattern="myapp/urls.py") # URL patterns

#### 2b. RLM Analysis
repl_load(pattern="myapp/**/*.py", var_name="app_code")
repl_exec(code="""
# Categorize by Django component
categories = {'models': [], 'views': [], 'serializers': [], 'tests': []}
for path in app_code.keys():
    if 'models.py' in path: categories['models'].append(path)
    elif 'views.py' in path: categories['views'].append(path)
    elif 'serializers.py' in path: categories['serializers'].append(path)
    elif 'test' in path.lower(): categories['tests'].append(path)

# Targeted analysis
analysis = {}
for path in categories['models']:
    analysis[path] = sub_llm(app_code[path], '''
        1. What Django models are defined? Fields and relationships?
        2. ISSUES: Missing migrations? Unindexed FKs? TODO/FIXME?
    ''')

# ... similar for views, serializers, etc.
""")

#### 2c. Record Findings
exam_record(
    component="myapp",
    summary="Django app for user management",
    connections=["authentication", "authorization"],
    issues=[...]
)
```

### 4.3 Customizing CLAUDE.md

The development guide needs project-specific sections:

```markdown
# CLAUDE.md - MyProject Development Guide

## Project Overview
MyProject is a [description]. Built with [stack].

## Quick Commands
```bash
# Build
npm run build

# Test
npm test

# Format
npm run format
```

## Architecture
[Your architecture here - update the diagram]

## TigerStyle Engineering Principles
[Keep or adapt these based on your project philosophy]

## EVI (Exploration & Verification Infrastructure)
[Keep this section - it's generic]

### Quick Reference
```bash
# Index the codebase
evi index build

# Start MCP server
evi serve
```

## Testing Guidelines
[Your project's testing standards]

## Code Style
[Your project's style guide]
```

### 4.4 Verification Framework Integration

EVI can integrate with different verification approaches:

| Project Type | Verification | Integration |
|--------------|--------------|-------------|
| **Distributed System** | DST | Add DST crate, `dst_run` tool |
| **Web App** | Property tests | `property_test_run` tool |
| **Library** | Fuzzing | `fuzz_run` tool |
| **Smart Contract** | Formal verification | `formal_verify` tool |

**Example: Adding Property Test Support**

```python
# evi/tools/handlers.py - add new tool

async def handle_property_test_run(self, arguments: dict) -> dict:
    """Run property tests with specific scenario."""
    scenario = arguments.get("scenario")
    seed = arguments.get("seed")

    # Run property tests (e.g., Hypothesis for Python)
    result = await self._run_command(
        f"pytest -k property --hypothesis-seed={seed} -v"
    )

    return {
        "success": result.returncode == 0,
        "output": result.stdout,
        "errors": result.stderr,
    }
```

---

## 5. Distribution Model

### 5.1 Open Source Package

**Recommended:** Distribute as open-source on PyPI

**Advantages:**
- Community contributions
- Wide adoption
- No licensing friction
- Ecosystem growth

**License Options:**
- **MIT** - Most permissive, allows commercial use
- **Apache 2.0** - Permissive with patent grant
- **GPL v3** - Copyleft, requires derivatives to be open source

### 5.2 Commercial Support Model

**Optional:** Offer paid support/services around open-source core

| Tier | What's Included |
|------|-----------------|
| **Free (Open Source)** | Core EVI package, templates, basic indexers |
| **Pro** | Language packs (Python, TS, Go), priority support |
| **Enterprise** | Custom indexers, on-prem deployment, SLA |

### 5.3 SaaS Model (Future)

**Potential:** Hosted EVI service

```
cloud.evi.dev
├── Project Setup (GUI)
├── Hosted MCP Server
├── Managed Indexes
├── Collaboration (team sessions)
└── Observability Integration (Tempo, Prometheus, Loki)
```

---

## 6. Example: Applying EVI to a New Project

### 6.1 Scenario: React + TypeScript Project

```bash
# 1. Install EVI
cd /path/to/react-project
npm install -g @evi/cli  # or: pip install evi

# 2. Initialize
evi init --language=typescript --framework=react

# Output:
# ✓ Detected package.json (TypeScript project)
# ✓ Detected React (found react in dependencies)
# ✓ Created .mcp.json
# ✓ Created CLAUDE.md (React/TypeScript template)
# ✓ Created .vision/CONSTRAINTS.md
# ✓ Built indexes:
#   - symbols.json (234 components, 89 hooks, 45 utils)
#   - modules.json (12 feature modules)
#   - dependencies.json (45 npm packages)
#   - tests.json (189 tests)

# 3. Customize
vim CLAUDE.md  # Add project-specific conventions
vim .vision/CONSTRAINTS.md  # Add project requirements

# 4. Start using
evi serve  # Starts MCP server

# 5. In Claude Code, use the skills:
# "Map this React codebase" → /codebase-map skill
# "How does auth work?" → /thorough-answer skill
```

### 6.2 Generated CLAUDE.md (React Template)

```markdown
# CLAUDE.md - React Project Development Guide

## Project Overview
[Auto-detected: React + TypeScript project]

## Quick Commands
```bash
npm run dev      # Start dev server
npm test         # Run tests
npm run build    # Production build
npm run lint     # ESLint
npm run format   # Prettier
```

## Architecture
[React-specific architecture section]

## Component Patterns
- Use functional components with hooks
- Co-locate tests with components
- Follow feature-based folder structure

## EVI Tools
```bash
evi index build    # Rebuild indexes
evi serve          # Start MCP server
```

## Skills Available
- `/codebase-map` - Map all components and hooks
- `/thorough-answer` - Answer questions thoroughly
```

---

## 7. Configuration Reference

### 7.1 .mcp.json (Generated by `evi init`)

```json
{
  "mcpServers": {
    "evi": {
      "command": "evi",
      "args": ["serve"],
      "env": {
        "EVI_CODEBASE_PATH": ".",
        "EVI_INDEX_PATH": ".evi-index",
        "EVI_SUB_LLM_MODEL": "claude-haiku-4-5-20251001",
        "ANTHROPIC_API_KEY": "${ANTHROPIC_API_KEY}"
      }
    }
  }
}
```

### 7.2 .evi/config.toml (Optional advanced config)

```toml
[project]
name = "MyProject"
language = "typescript"
framework = "react"

[indexer]
path = ".evi-index"
exclude = ["node_modules", "dist", "build"]
include = ["src/**/*.ts", "src/**/*.tsx"]

[indexer.typescript]
symbol_types = ["interface", "class", "function", "const"]
parse_jsx = true

[server]
port = 3000
host = "localhost"

[verification]
framework = "jest"
property_tests = true

[observability]
# Future: production integration
enabled = false
traces_endpoint = "http://localhost:3200"
metrics_endpoint = "http://localhost:9090"
```

---

## 8. Roadmap for Portability

### Phase 1: Core Package (Current)
- [x] Extract MCP server into standalone package
- [x] Create templates for common scenarios
- [ ] Package on PyPI as `evi`

### Phase 2: Language Support
- [ ] Add Python indexer
- [ ] Add TypeScript indexer
- [ ] Add Go indexer

### Phase 3: Framework Templates
- [ ] React template
- [ ] Django template
- [ ] Next.js template
- [ ] Express template

### Phase 4: CLI Tool
- [ ] `evi init` command
- [ ] `evi index build` command
- [ ] `evi serve` command
- [ ] `evi upgrade` command

### Phase 5: Distribution
- [ ] Publish to PyPI
- [ ] Create Docker image
- [ ] NPM wrapper (`npx create-evi-project`)
- [ ] GitHub template repository

---

## 9. Contributing

### 9.1 Adding Language Support

To add a new language indexer:

1. Create `evi/indexer/languages/yourlang.py`
2. Implement `YourLangIndexer` class
3. Add tree-sitter parser dependency
4. Update `indexer.py` to register the language
5. Add tests in `tests/test_indexer_yourlang.py`
6. Submit PR

### 9.2 Adding Skills

To contribute a new skill:

1. Create skill directory in `templates/skills/`
2. Write `SKILL.md` with workflow
3. Add language/framework-specific examples
4. Test on real projects
5. Document in `docs/skills.md`
6. Submit PR

---

## Appendix A: Project Type Matrix

| Project Type | Language | Indexer | Skills | Verification |
|--------------|----------|---------|--------|--------------|
| Distributed System | Rust | ✅ | codebase-map, thorough-answer | DST |
| Web API | Rust (Axum) | ✅ | codebase-map, api-endpoints | Property tests |
| Web API | Python (FastAPI) | ⚠️ | codebase-map, api-endpoints | pytest |
| Web App | TypeScript (React) | ⚠️ | codebase-map, component-map | Jest |
| CLI Tool | Go | ⚠️ | codebase-map | go test |
| Library | Rust | ✅ | codebase-map, api-surface | cargo test |

✅ = Supported now
⚠️ = Planned
❌ = Not planned

---

*This document defines how EVI can be packaged and applied to any project.*
