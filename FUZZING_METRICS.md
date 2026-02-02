# Fuzzing Campaign Metrics

This fuzzer tracks two critical metrics to measure fuzzing effectiveness:

1. **SCALE Decode Success Rate** - How often does random input decode to valid runtime calls?
2. **Call Filter Rate** - What percentage of valid calls are filtered out before execution?

These metrics answer: *"Is the fuzzer actually testing the runtime, or just spinning its wheels?"*

## What Gets Measured

### Per-Iteration Decode Statistics (`decode_stats` table)

Every fuzzer iteration records:
- **total_attempts**: How many times we tried to decode a call from the input bytes
- **successful_decodes**: How many decoded to valid `RuntimeCall` structs
- **filtered_calls**: How many valid calls were rejected by `call_filter()`

**Actual executions** = `successful_decodes - filtered_calls`

### Per-Execution Records (`executions` table)

Every runtime call that actually executes records:
- **call_variant**: The pallet/call type (e.g., "Balances", "Timestamp")
- **args_json**: Full debug representation
- **origin**: Which account signed the call
- **result**: "Ok" or "Err(...)"

## Quick Health Check

Run these queries to assess fuzzer effectiveness:

```sql
-- Campaign summary
SELECT
    COUNT(*) as iterations,
    SUM(successful_decodes - filtered_calls) as actual_executions,
    ROUND(SUM(successful_decodes) * 100.0 / SUM(total_attempts), 1) as decode_rate_pct,
    ROUND(SUM(filtered_calls) * 100.0 / SUM(successful_decodes), 1) as filter_rate_pct,
    ROUND((SUM(successful_decodes) - SUM(filtered_calls)) * 100.0 / SUM(total_attempts), 2) as execution_rate_pct
FROM decode_stats;
```

**What the numbers mean:**

- **decode_rate_pct**: How often random input bytes decode to valid calls
- **filter_rate_pct**: How many valid calls are rejected before execution
- **execution_rate_pct**: Bottom line - what % of iterations actually test the runtime

```sql
-- What calls are actually executing?
SELECT
    call_variant,
    COUNT(*) as count,
    SUM(CASE WHEN result = 'Ok' THEN 1 ELSE 0 END) as successes,
    SUM(CASE WHEN result != 'Ok' THEN 1 ELSE 0 END) as failures
FROM executions
GROUP BY call_variant
ORDER BY count DESC;
```

```sql
-- Most common errors
SELECT
    result,
    COUNT(*) as count
FROM executions
WHERE result != 'Ok'
GROUP BY result
ORDER BY count DESC
LIMIT 10;
```

## Understanding the Numbers

### decode_rate_pct
Percentage of decode attempts that produce valid `RuntimeCall` structs. Depends on:
- Corpus quality (seeded with real extrinsics vs. random bytes)
- `RuntimeCall` complexity (larger enums are harder to fuzz)
- Campaign duration (coverage-guided fuzzing learns over time)

### filter_rate_pct
Percentage of valid calls rejected by `call_filter()` before execution. Check [templates/kitchensink/src/main.rs:295-354](templates/kitchensink/src/main.rs#L295-L354) to see what's being filtered and why.

### execution_rate_pct
Bottom line: what percentage of fuzzer iterations actually test the runtime. Everything else is wasted effort.

## Setup

### Building with Metrics

Metrics are enabled by default:

```bash
SKIP_WASM_BUILD=1 cargo ziggy build
```

Disable with:

```bash
SKIP_WASM_BUILD=1 cargo ziggy build --no-default-features
```

### Running with Metrics

Set `TELEMETRY_DB` environment variable:

```bash
TELEMETRY_DB=./fuzzing_data.db SKIP_WASM_BUILD=1 cargo ziggy fuzz
```

Parallel fuzzing (4 workers):

```bash
TELEMETRY_DB=./fuzzing_data.db SKIP_WASM_BUILD=1 cargo ziggy fuzz --no-honggfuzz -j 4
```

If `TELEMETRY_DB` is not set, metrics are disabled (zero overhead).

## Data Collection Details

### Batching

The two tables have different flushing strategies:

**Execution records** (`executions` table):
- Flush **immediately** on every runtime call execution
- No batching (changed from original design to ensure data is captured)
- Every execution writes to DB in real-time

**Decode statistics** (`decode_stats` table):
- Accumulate until 100 iterations
- Flush to database in single transaction
- On graceful exit, remaining records (<100) are flushed

**Implication**: Execution records appear in the database immediately. Decode stats appear in batches of 100 iterations.

Check buffered decode stats:
```sql
SELECT
    SUM(successful_decodes) - SUM(filtered_calls) - (SELECT COUNT(*) FROM executions) as buffered_executions
FROM decode_stats;
```
A positive number means decode stats are buffered but not yet flushed (will show after 100 more iterations).

### Data Loss Window

On crash/kill -9:
- **~0 execution records lost** (immediate flush)
- **Up to 99 decode stat iterations lost** (batched)

For statistical analysis over millions of iterations, losing <100 decode stat records is negligible.

### Parallel Fuzzing

All workers write to the same database. SQLite uses file locks for concurrency.

**You may see**: `Telemetry: Failed to start transaction: database is locked`

This is expected with immediate execution flushing. The execution record is discarded on lock failure (logged to stderr).

**Reducing lock contention**:
- Use separate DB per worker: `TELEMETRY_DB=./fuzz_worker_${WORKER_ID}.db`
- Merge databases post-campaign with:
  ```bash
  sqlite3 merged.db "ATTACH 'worker_1.db' AS w1; INSERT INTO executions SELECT * FROM w1.executions;"
  ```
- Accept some data loss from lock failures (decode stats provide execution counts anyway)

### Performance Overhead

**Memory**: Minimal - small buffers for decode stats only

**CPU**:
- Serializing debug strings for executed calls (already done for logging)
- Counting decode attempts (3 integers per iteration)

**I/O**:
- **1 transaction per execution** (immediate flush to ensure data capture)
- 1 transaction per 100 iterations (decode stats batched)

Overhead depends on execution rate. With immediate flushing, expect:
- **High decode rate** (many executions): ~5-10% overhead from frequent DB writes
- **Low decode rate** (few executions): <1% overhead

Trade-off: We prioritize data capture over performance. Original batched approach lost data on crashes.

## Database Schema

```sql
CREATE TABLE IF NOT EXISTS executions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    call_variant TEXT NOT NULL,
    args_json TEXT NOT NULL,
    origin TEXT NOT NULL,
    result TEXT NOT NULL,
    timestamp INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_call_variant ON executions(call_variant);
CREATE INDEX IF NOT EXISTS idx_result ON executions(result);
CREATE INDEX IF NOT EXISTS idx_exec_timestamp ON executions(timestamp);

CREATE TABLE IF NOT EXISTS decode_stats (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    total_attempts INTEGER NOT NULL,
    successful_decodes INTEGER NOT NULL,
    filtered_calls INTEGER NOT NULL,
    timestamp INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);
```

Timestamps are set automatically by SQLite on insert (Unix epoch seconds).

## Essential Queries

Copy-paste these into your terminal with `sqlite3 -header -column <path_to_db> "<query>"` to get readable output with column names.

### Campaign status
```sql
SELECT
    COUNT(*) as iterations,
    SUM(successful_decodes - filtered_calls) as actual_executions,
    ROUND(SUM(successful_decodes) * 100.0 / SUM(total_attempts), 1) as decode_rate_pct,
    ROUND(SUM(filtered_calls) * 100.0 / SUM(successful_decodes), 1) as filter_rate_pct,
    ROUND((SUM(successful_decodes) - SUM(filtered_calls)) * 100.0 / SUM(total_attempts), 2) as execution_rate_pct
FROM decode_stats;
```

### How many decode stats are buffered?
```sql
SELECT COUNT(*) FROM executions;
```
Compare this to `actual_executions` from the status query. Difference = decode stat iterations not yet flushed (batched at 100 records).

### What calls executed?
```sql
SELECT call_variant, COUNT(*) as count
FROM executions
GROUP BY call_variant
ORDER BY count DESC;
```

### DB file size
```bash
ls -lh fuzzing_data.db
```
