use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

#[test]
fn closed_seat_drops_idle_sse_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut buffer = [0; 2048];
        assert!(stream.read(&mut buffer).unwrap() > 0);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
            .unwrap();
        ready_tx.send(()).unwrap();
        stream.read(&mut buffer).unwrap()
    });
    let seat = Arc::new(Mutex::new(Seat::default()));
    let worker_seat = seat.clone();
    let worker = thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_sse(worker_seat, format!("http://{address}")));
    });
    ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !seat.lock().unwrap().streaming {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    seat.lock().unwrap().closed = true;
    assert_eq!(server.join().unwrap(), 0);
    worker.join().unwrap();
    assert!(!seat.lock().unwrap().streaming);
}
