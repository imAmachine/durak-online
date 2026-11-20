use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[test]
fn connect_waits_for_initial_state_and_prefers_explicit_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut streams = Vec::new();
        for expected in [
            "id=envid&token=envtoken",
            "id=explicitid&token=explicittoken",
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let len = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..len]).contains(expected));
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n").unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(150));
            write!(stream, "data: {{\"type\":\"account\",\"account\":{{\"name\":\"player\"}}}}\n\ndata: {{\"type\":\"game\",\"state\":{{\"room\":{{\"id\":\"ROOM\",\"status\":\"waiting\"}}}}}}\n\n").unwrap();
            stream.flush().unwrap();
            streams.push(stream);
        }
        thread::sleep(Duration::from_secs(1));
    });
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sessions = std::env::temp_dir().join(format!(
        "durak-connect-test-{}-{timestamp}.json",
        std::process::id()
    ));
    let mut child = Command::new(env!("CARGO_BIN_EXE_platform-mcp"))
        .env("DURAK_URL", format!("http://{address}"))
        .env("DURAK_MCP_SESSIONS", &sessions)
        .env("DURAK_CLIENT_ID", "envid")
        .env("DURAK_TOKEN", "envtoken")
        .env_remove("DURAK_LOGIN")
        .env_remove("DURAK_DISCORD_ACCOUNTS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    for args in [
        json!({"seat":"env"}),
        json!({"seat":"explicit","clientId":"explicitid","token":"explicittoken"}),
    ] {
        writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"durak_connect","arguments":args}})).unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        let content = &response["result"]["structuredContent"];
        assert_eq!(content["account"]["name"], "player", "{response}");
        assert_eq!(content["room"]["id"], "ROOM", "{response}");
        assert_eq!(content["status"], "waiting", "{response}");
    }
    child.kill().unwrap();
    child.wait().unwrap();
    server.join().unwrap();
    let _ = std::fs::remove_file(sessions);
}
