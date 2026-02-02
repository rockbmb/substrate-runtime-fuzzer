use rusqlite::{params, Connection};
use serde_json::json;
use solochain_template_runtime::{AccountId, RuntimeCall};
use sp_runtime::DispatchResultWithInfo;
use std::sync::Mutex;

const BATCH_SIZE: usize = 100;

struct ExecutionRecord {
    call_variant: String,
    args_json: String,
    origin: String,
    result: String,
}

pub struct TelemetryLogger {
    conn: Mutex<Connection>,
    buffer: Mutex<Vec<ExecutionRecord>>,
}

impl TelemetryLogger {
    pub fn new(db_path: &str) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS executions (
                id INTEGER PRIMARY KEY,
                call_variant TEXT NOT NULL,
                args_json TEXT NOT NULL,
                origin TEXT NOT NULL,
                result TEXT NOT NULL
            )",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            buffer: Mutex::new(Vec::with_capacity(BATCH_SIZE)),
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

        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.push(record);

            if buffer.len() >= BATCH_SIZE {
                self.flush_buffer(&mut buffer);
            }
        }
    }

    fn flush_buffer(&self, buffer: &mut Vec<ExecutionRecord>) {
        if buffer.is_empty() {
            return;
        }

        if let Ok(mut conn) = self.conn.lock() {
            match conn.transaction() {
                Ok(tx) => {
                    for record in buffer.drain(..) {
                        if let Err(e) = tx.execute(
                            "INSERT INTO executions (call_variant, args_json, origin, result) VALUES (?1, ?2, ?3, ?4)",
                            params![&record.call_variant, &record.args_json, &record.origin, &record.result],
                        ) {
                            eprintln!("Telemetry: Failed to insert execution record: {}", e);
                            break;
                        }
                    }

                    if let Err(e) = tx.commit() {
                        eprintln!("Telemetry: Failed to commit executions: {}", e);
                    }
                }
                Err(e) => eprintln!("Telemetry: Failed to start transaction: {}", e),
            }
        }
    }
}

impl Drop for TelemetryLogger {
    fn drop(&mut self) {
        // Flush any remaining records
        if let Ok(mut buffer) = self.buffer.lock() {
            self.flush_buffer(&mut buffer);
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
