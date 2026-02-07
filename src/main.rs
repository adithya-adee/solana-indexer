#![warn(clippy::all, clippy::pedantic)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv()?;

    Ok(())
}
