mod presentation;
mod protocol;
mod seat;
mod strategy;
mod tools;
mod transport;

fn main() -> std::io::Result<()> {
    seat::restore_sessions();
    let restored: Vec<_> = seat::seats().lock().unwrap().values().cloned().collect();
    for seat in &restored {
        transport::start_sse(seat.clone());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if restored.iter().all(|seat| {
            let seat = seat.lock().unwrap();
            seat.streaming && (seat.game.is_some() || seat.lobby.is_some() || seat.noroom)
        }) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    protocol::run()
}
