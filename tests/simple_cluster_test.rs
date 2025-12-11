//! Simple test to verify cluster commands work

#![cfg(feature = "cluster")]

use redis::Client;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn cluster_commands_work() -> Result<(), Box<dyn std::error::Error>> {
    use tokio::process::Command;
    use tempfile::TempDir;

    let data_dir = TempDir::new()?;
    let config_path = data_dir.path().join("config.toml");

    let config_contents = format!(
        "[server]\nhost = \"127.0.0.1\"\nport = 7000\n\n[storage]\nengine = \"aidb\"\ndata_dir = \"{}\"\ndatabases = 4\n\n[logging]\nlevel = \"debug\"\n",
        data_dir.path().display()
    );

    std::fs::write(&config_path, config_contents)?;

    // Start server
    let mut child = Command::new("./target/debug/aikv")
        .env("AIKV_BOOTSTRAP", "1")
        .env("AIKV_NODE_ID", "1")
        .arg("--config")
        .arg(&config_path)
        .spawn()?;

    // Wait for server to start
    sleep(Duration::from_secs(3)).await;

    // Connect and test
    let client = Client::open("redis://127.0.0.1:7000/")?;
    let mut con = client.get_connection()?;

    // Test PING
    let ping: String = redis::cmd("PING").query(&mut con)?;
    println!("PING: {}", ping);
    assert_eq!(ping, "PONG");

    // Test CLUSTER MYID
    let myid: String = redis::cmd("CLUSTER").arg("MYID").query(&mut con)?;
    println!("CLUSTER MYID: {}", myid);

    // Test CLUSTER NODES
    let nodes: String = redis::cmd("CLUSTER").arg("NODES").query(&mut con)?;
    println!("CLUSTER NODES:\n{}", nodes);

    child.kill().await?;
    Ok(())
}
