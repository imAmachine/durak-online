use std::{env, error::Error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port = env::var("PORT").unwrap_or_else(|_| "4173".into());
    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}")).await?;
    eprintln!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, platform_api::router()).await?;
    Ok(())
}
