use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = env::temp_dir().join(format!(
            "durak-mcp-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!(
                "failed to remove fixture directory {}: {error}",
                self.0.display()
            );
        }
    }
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let pid = self.0.id();
        match self.0.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(error) => eprintln!("failed to check process {pid}: {error}"),
        }
        if let Err(error) = self.0.kill() {
            eprintln!("failed to terminate process {pid}: {error}");
            return;
        }
        if let Err(error) = self.0.wait() {
            eprintln!("failed to reap process {pid}: {error}");
        }
    }
}

fn post(client: &Client, url: &str, body: Value) -> Value {
    client
        .post(url)
        .json(&body)
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap()
}

fn clean_env(command: &mut Command) -> &mut Command {
    command
        .env_remove("DURAK_LOGIN")
        .env_remove("DURAK_PASSWORD")
        .env_remove("DURAK_ACCOUNTS")
        .env_remove("DURAK_CLIENT_ID")
        .env_remove("DURAK_TOKEN")
        .env_remove("DURAK_ADMIN_KEY")
        .env_remove("DURAK_DISCORD_CLIENT_ID")
        .env_remove("DURAK_DISCORD_TOKEN")
        .env_remove("DURAK_DISCORD_ACCOUNTS")
        .env_remove("DURAK_MIGRATIONS_DIR")
        .env_remove("ADMIN_KEY")
        .env_remove("DISCORD_CLIENT_ID")
        .env_remove("DISCORD_CLIENT_SECRET")
        .env_remove("DISCORD_REDIRECT_URI")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
}

fn read_response(output: &Receiver<std::io::Result<String>>) -> Value {
    let line = output
        .recv_timeout(Duration::from_secs(120))
        .expect("timed out waiting for MCP response")
        .expect("failed to read MCP response");
    serde_json::from_str(&line).expect("invalid MCP response")
}

#[test]
fn stdio_game_flow_with_rust_backend_fixture() {
    let temp = TempDir::new();
    let db = temp.0.join("durak.sqlite");
    let server_bin = env::var_os("DURAK_SERVER_BIN")
        .expect("DURAK_SERVER_BIN must point to a built platform-server binary");
    let mut server = Kill(
        clean_env(&mut Command::new(server_bin))
            .env("HOST", "127.0.0.1")
            .env("PORT", "0")
            .env("DURAK_DB", &db)
            .env("ADMIN_KEY", "service-test-admin")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Rust backend is required for the service integration test"),
    );
    let (address_tx, address_rx) = mpsc::channel();
    let stderr = server.0.stderr.take().unwrap();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            eprintln!("backend: {line}");
            if let Some(address) = line.strip_prefix("Listening on ") {
                let _ = address_tx.send(address.to_owned());
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let address = address_rx
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .expect("backend did not report its listening address within 60 seconds");
    let client = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{address}");
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "Rust backend did not become ready within 60 seconds"
        );
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "Rust backend exited before becoming ready"
        );
        if client
            .get(format!("{base}/readyz"))
            .timeout(remaining.min(Duration::from_secs(1)))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            break;
        }
        thread::sleep(remaining.min(Duration::from_millis(20)));
    }
    let mut accounts = serde_json::Map::new();
    for (seat, name, password) in [
        ("a", "service-a", "service-a-password"),
        ("b", "service-b", "service-b-password"),
        ("auto1", "service-auto1", "service-auto1-password"),
        ("auto2", "service-auto2", "service-auto2-password"),
    ] {
        post(
            &client,
            &format!("{base}/api/admin"),
            json!({"key":"service-test-admin","op":"create_temp","name":name,"password":password}),
        );
        let session = post(
            &client,
            &format!("{base}/api/auth/temp/login"),
            json!({"name":name,"password":password}),
        );
        accounts.insert(
            seat.to_owned(),
            json!({"clientId":session["clientId"],"token":session["token"]}),
        );
    }
    let session_file = temp.0.join("sessions.json");
    let mut mcp = Kill(
        clean_env(&mut Command::new(env!("CARGO_BIN_EXE_platform-mcp")))
            .env("DURAK_MCP_SESSIONS", &session_file)
            .env("DURAK_URL", &base)
            .env(
                "DURAK_DISCORD_ACCOUNTS",
                Value::Object(accounts).to_string(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = mcp.0.stdin.take().unwrap();
    let stdout = mcp.0.stdout.take().unwrap();
    let (output_tx, output) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if output_tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut call = |name: &str, args: Value| -> Value {
        writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
        input.flush().unwrap();
        let response = read_response(&output);
        assert_eq!(response["id"], 1, "{name}: {response}");
        assert!(response["result"]["isError"] != true, "{name}: {response}");
        response["result"]["structuredContent"].clone()
    };
    call("durak_connect", json!({"seat":"a"}));
    call("durak_connect", json!({"seat":"b"}));
    let room = call("durak_room_create", json!({"seat":"a"}));
    call("durak_room_join", json!({"seat":"b","code":room["code"]}));
    call("durak_start", json!({"seat":"a"}));
    let state = call("durak_state", json!({"seat":"a"}));
    assert_eq!(state["state"]["room"]["status"], "playing");
    assert_eq!(state["state"]["hand"].as_array().unwrap().len(), 6);
    let a = call("durak_state", json!({"seat":"a"}));
    let attacker = if a["state"]["canAttack"] == true {
        "a"
    } else {
        "b"
    };
    let state = call("durak_state", json!({"seat":attacker}));
    let card = state["state"]["hand"][0]["id"].clone();
    let played = call(
        "durak_action",
        json!({"seat":attacker,"action":"attack","cardId":card}),
    );
    assert_eq!(
        played["snapshot"]["table"],
        format!("{}!", card.as_str().unwrap())
    );
    let hint = call("durak_hints", json!({"seat":attacker}));
    assert!(hint["attack"].is_array());
    let auto = call("durak_autoplay", json!({"maxSteps":1000}));
    assert_eq!(auto["finished"], true, "{auto}");
    assert!(auto["steps"].as_u64().unwrap() > 0);
    drop(call);
    for (id, seat) in [(2, "a"), (3, "b")] {
        writeln!(input,"{}",json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"durak_follow","arguments":{"seat":seat,"totalMs":30000,"maxSteps":1000}}})).unwrap();
    }
    input.flush().unwrap();
    let mut ids = Vec::new();
    for _ in 0..2 {
        let response = read_response(&output);
        ids.push(response["id"].as_u64().expect("missing follow response id"));
        assert_eq!(
            response["result"]["structuredContent"]["finished"], true,
            "{response}"
        );
    }
    ids.sort_unstable();
    assert_eq!(ids, [2, 3]);
}
