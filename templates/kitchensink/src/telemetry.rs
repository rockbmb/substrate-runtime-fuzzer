//! Telemetry module for capturing fuzzing campaign data.
//!
//! This module provides batched database writes for two types of records:
//! - **Execution records**: Details of each runtime call execution (variant, args, origin, result)
//! - **Decode statistics**: Per-iteration stats on SCALE decoding success/failure rates
//!
//! Both record types use batched writes (100 records per transaction) to minimize
//! database overhead and avoid locking issues during parallel fuzzing.
//!
//! Database errors are logged to stderr but never crash the fuzzer.

use rusqlite::{params, Connection};
use serde_json::json;
use kitchensink_runtime::{AccountId, RuntimeCall};
use sp_runtime::DispatchResultWithInfo;
use std::sync::Mutex;

const BATCH_SIZE: usize = 100;
const DECODE_STATS_BATCH_SIZE: usize = 100;

struct ExecutionRecord {
    call_variant: String,
    args_json: String,
    origin: String,
    result: String,
}

struct DecodeStatsRecord {
    total_attempts: usize,
    successful_decodes: usize,
    filtered_calls: usize,
}

pub struct TelemetryLogger {
    conn: Mutex<Connection>,
    exec_buffer: Mutex<Vec<ExecutionRecord>>,
    decode_buffer: Mutex<Vec<DecodeStatsRecord>>,
}

impl TelemetryLogger {
    pub fn new(db_path: &str) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS executions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                call_variant TEXT NOT NULL,
                args_json TEXT NOT NULL,
                origin TEXT NOT NULL,
                result TEXT NOT NULL,
                timestamp INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS decode_stats (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                total_attempts INTEGER NOT NULL,
                successful_decodes INTEGER NOT NULL,
                filtered_calls INTEGER NOT NULL,
                timestamp INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Create indexes for common queries
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_call_variant ON executions(call_variant)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_result ON executions(result)", []);
        let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_exec_timestamp ON executions(timestamp)", []);

        Ok(Self {
            conn: Mutex::new(conn),
            exec_buffer: Mutex::new(Vec::with_capacity(BATCH_SIZE)),
            decode_buffer: Mutex::new(Vec::with_capacity(DECODE_STATS_BATCH_SIZE)),
        })
    }

    pub fn log_execution(
        &self,
        call_debug: &str,
        origin: AccountId,
        result: &DispatchResultWithInfo<sp_runtime::traits::PostDispatchInfoOf<RuntimeCall>>,
    ) {
        let record = ExecutionRecord {
            call_variant: extract_call_variant(call_debug),
            args_json: json!({"debug": call_debug}).to_string(),
            origin: format!("{:?}", origin),
            result: format_dispatch_result(result),
        };

        if let Ok(mut buffer) = self.exec_buffer.lock() {
            buffer.push(record);

            if buffer.len() >= BATCH_SIZE {
                self.flush_exec_buffer(&mut buffer);
            }
        }
    }

    pub fn log_decode_stats(&self, total_attempts: usize, successful: usize, filtered: usize) {
        let record = DecodeStatsRecord {
            total_attempts,
            successful_decodes: successful,
            filtered_calls: filtered,
        };

        if let Ok(mut buffer) = self.decode_buffer.lock() {
            buffer.push(record);

            if buffer.len() >= DECODE_STATS_BATCH_SIZE {
                self.flush_decode_buffer(&mut buffer);
            }
        }
    }

    fn flush_exec_buffer(&self, buffer: &mut Vec<ExecutionRecord>) {
        if buffer.is_empty() {
            return;
        }

        if let Ok(mut conn) = self.conn.lock() {
            match conn.transaction() {
                Ok(tx) => {
                    let mut failed = false;
                    for record in buffer.iter() {
                        if let Err(e) = tx.execute(
                            "INSERT INTO executions (call_variant, args_json, origin, result) VALUES (?1, ?2, ?3, ?4)",
                            params![&record.call_variant, &record.args_json, &record.origin, &record.result],
                        ) {
                            eprintln!("Telemetry: Failed to insert execution record: {}", e);
                            failed = true;
                            break;
                        }
                    }

                    if !failed {
                        match tx.commit() {
                            Ok(_) => buffer.clear(),
                            Err(e) => eprintln!("Telemetry: Failed to commit executions: {}", e),
                        }
                    }
                }
                Err(e) => eprintln!("Telemetry: Failed to start transaction for executions: {}", e),
            }
        }
    }

    fn flush_decode_buffer(&self, buffer: &mut Vec<DecodeStatsRecord>) {
        if buffer.is_empty() {
            return;
        }

        if let Ok(mut conn) = self.conn.lock() {
            match conn.transaction() {
                Ok(tx) => {
                    let mut failed = false;
                    for record in buffer.iter() {
                        if let Err(e) = tx.execute(
                            "INSERT INTO decode_stats (total_attempts, successful_decodes, filtered_calls) VALUES (?1, ?2, ?3)",
                            params![record.total_attempts as i64, record.successful_decodes as i64, record.filtered_calls as i64],
                        ) {
                            eprintln!("Telemetry: Failed to insert decode stats: {}", e);
                            failed = true;
                            break;
                        }
                    }

                    if !failed {
                        match tx.commit() {
                            Ok(_) => buffer.clear(),
                            Err(e) => eprintln!("Telemetry: Failed to commit decode stats: {}", e),
                        }
                    }
                }
                Err(e) => eprintln!("Telemetry: Failed to start transaction for decode stats: {}", e),
            }
        }
    }
}

impl Drop for TelemetryLogger {
    fn drop(&mut self) {
        // Flush any remaining records
        if let Ok(mut buffer) = self.exec_buffer.lock() {
            self.flush_exec_buffer(&mut buffer);
        }
        if let Ok(mut buffer) = self.decode_buffer.lock() {
            self.flush_decode_buffer(&mut buffer);
        }
    }
}

fn extract_call_variant(call_debug: &str) -> String {
    // Extract variant from debug string like "Balances(transfer { ... })"
    if let Some(pos) = call_debug.find('(') {
        call_debug[..pos].to_string()
    } else {
        call_debug.to_string()
    }
}

fn format_dispatch_result(
    result: &DispatchResultWithInfo<sp_runtime::traits::PostDispatchInfoOf<RuntimeCall>>,
) -> String {
    match result {
        Ok(_) => "Ok".to_string(),
        Err(e) => format!("Err({:?})", e.error),
    }
}
