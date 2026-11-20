use crate::{presentation::error, tools::tool};
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Write},
    sync::{Arc, Mutex},
    thread,
};
fn annotations(name: &str) -> Value {
    match name {
        "durak_connect" => {
            json!({"readOnlyHint":false,"destructiveHint":false,"idempotentHint":false,"openWorldHint":true})
        }
        "durak_disconnect" => {
            json!({"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false})
        }
        "durak_room_leave" => {
            json!({"readOnlyHint":false,"destructiveHint":true,"idempotentHint":true,"openWorldHint":false})
        }
        "durak_admin" => {
            json!({"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":true})
        }
        "durak_autoplay" => {
            json!({"readOnlyHint":false,"destructiveHint":false,"idempotentHint":false,"openWorldHint":true})
        }
        "durak_room_create" | "durak_room_join" | "durak_start" | "durak_action"
        | "durak_follow" | "durak_profile" => {
            json!({"readOnlyHint":false,"destructiveHint":false,"idempotentHint":false,"openWorldHint":false})
        }
        _ => {
            json!({"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false})
        }
    }
}

fn response(request: Value) -> Option<Value> {
    let obj = match request.as_object() {
        Some(o) => o,
        None => {
            return Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}}),
            )
        }
    };
    let id = obj.get("id").cloned();
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || obj.get("method").and_then(Value::as_str).is_none()
    {
        return Some(
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"Invalid Request"}}),
        );
    }
    let id = id?;
    let method = obj["method"].as_str().unwrap();
    let value = match method {
        "initialize" => {
            let req = request["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2024-11-05");
            let v = if ["2024-11-05", "2025-03-26", "2025-06-18"].contains(&req) {
                req
            } else {
                "2024-11-05"
            };
            json!({"protocolVersion":v,"capabilities":{"tools":{"listChanged":false},"resources":{},"prompts":{}},"serverInfo":{"name":"durak","version":"1.1.0"}})
        }
        "ping" => json!({}),
        "resources/list" => json!({"resources": []}),
        "prompts/list" => json!({"prompts": []}),
        "tools/list" => {
            let mut tools: Value = serde_json::from_str(include_str!("../tools.json")).unwrap();
            if let Some(items) = tools.as_array_mut() {
                for item in items {
                    if let Some(name) = item["name"].as_str() {
                        item["annotations"] = annotations(name);
                    }
                }
            }
            json!({"tools":tools})
        }
        "tools/call" => {
            let name = request["params"]["name"].as_str().unwrap_or("");
            let tools: Value = serde_json::from_str(include_str!("../tools.json")).unwrap();
            if !tools.as_array().unwrap().iter().any(|t| t["name"] == name) {
                return Some(
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Unknown or missing tool name"}}),
                );
            }
            let args = request["params"]
                .get("arguments")
                .cloned()
                .unwrap_or(json!({}));
            tool(name, &args).unwrap_or_else(error)
        }
        _ => {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
            )
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":value}))
}
pub(super) fn run() -> io::Result<()> {
    let out = Arc::new(Mutex::new(io::BufWriter::new(io::stdout())));
    let mut jobs = Vec::new();
    for line in io::stdin().lock().lines() {
        let line = line?;
        let out = out.clone();
        jobs.push(thread::spawn(move || {
            let reply = match serde_json::from_str(&line) {
                Ok(v) => response(v),
                Err(_) => Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}})),
            };
            if let Some(v) = reply {
                let mut w = out.lock().unwrap();
                let _ = serde_json::to_writer(&mut *w, &v);
                let _ = w.write_all(b"\n");
                let _ = w.flush();
            }
        }));
    }
    for job in jobs {
        let _ = job.join();
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol() {
        assert!(response(json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        let initialized = response(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}})).unwrap();
        assert_eq!(
            initialized["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
        assert_eq!(initialized["result"]["serverInfo"]["version"], "1.1.0");
        let listed = response(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap();
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 19);
        assert_eq!(
            tools.iter().find(|t| t["name"] == "durak_state").unwrap()["annotations"]
                ["readOnlyHint"],
            true
        );
        assert_eq!(
            tools.iter().find(|t| t["name"] == "durak_admin").unwrap()["annotations"]
                ["destructiveHint"],
            true
        );
        assert_eq!(
            response(json!({"jsonrpc":"2.0","id":1,"method":"resources/list"})).unwrap()["result"]
                ["resources"],
            json!([])
        );
        assert_eq!(
            response(json!({"jsonrpc":"2.0","id":1,"method":"prompts/list"})).unwrap()["result"]
                ["prompts"],
            json!([])
        );
        assert_eq!(
            response(json!({"jsonrpc":"2.0","id":1,"method":"bogus"})).unwrap()["error"]["code"],
            -32601
        );
    }
}
