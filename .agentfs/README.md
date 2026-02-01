# AgentFS - Agent State Storage

SQLite-backed storage for agent state and audit trail. This directory is git-ignored and contains ephemeral agent memory.

## Database Schema

The `agent.db` SQLite database contains:

### Tables

#### `agent_state`
Agent's current working context:
- `key` (TEXT PRIMARY KEY) - State key (e.g., "current_task", "plan")
- `value` (TEXT) - State value (JSON)
- `updated_at` (TEXT) - ISO 8601 timestamp

#### `verified_facts`
Facts verified by execution (not by reading docs):
- `id` (INTEGER PRIMARY KEY)
- `claim` (TEXT) - The claim being verified
- `method` (TEXT) - How it was verified (e.g., "test execution", "cargo clippy")
- `result` (TEXT) - The verification result (JSON)
- `verified_at` (TEXT) - ISO 8601 timestamp

#### `audit_log`
Audit trail of all tool calls:
- `id` (INTEGER PRIMARY KEY)
- `timestamp` (TEXT) - ISO 8601 timestamp
- `tool` (TEXT) - Tool name
- `args` (TEXT) - Tool arguments (JSON)
- `result` (TEXT) - Tool result (JSON)
- `git_sha` (TEXT) - Git SHA at time of call

## Inspecting the Database

```bash
# Open SQLite shell
sqlite3 .agentfs/agent.db

# View current task
SELECT * FROM agent_state WHERE key = 'current_task';

# View recent verified facts
SELECT claim, method, verified_at FROM verified_facts ORDER BY verified_at DESC LIMIT 10;

# View audit log
SELECT timestamp, tool, args FROM audit_log ORDER BY timestamp DESC LIMIT 20;
```

## Usage via MCP Tools

(Phase 4 - not yet implemented)

```javascript
// Get state
mcp.state_get('current_task')

// Set state
mcp.state_set('current_task', {description: 'Building indexes', phase: 1})

// Start task
mcp.state_task_start('Build symbol index')

// Store verified fact
mcp.state_verified_fact('All tests pass', 'cargo test', {exit_code: 0, tests_run: 74})

// Complete task
mcp.state_task_complete({proof: 'test_output_sha256'})
```

## Maintenance

This database is ephemeral and can be deleted at any time. It will be recreated automatically. The database should be git-ignored.
