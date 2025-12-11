//! Cross-process MetaRaft convergence test.
//!
//! Spins up two AiKv binaries (with cluster feature) backed by AiDb storage
//! and verifies cluster membership converges via MetaRaft across processes.

#![cfg(feature = "cluster")]

use anyhow::{bail, Result};
use redis::Client;
use std::fs;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::process::{Child, Command};
use tokio::time::sleep;

struct NodeHandle {
	child: Child,
	data_dir: TempDir,
	port: u16,
}

impl NodeHandle {
	async fn kill(mut self) {
		let _ = self.child.kill().await;
	}
}

impl Drop for NodeHandle {
	fn drop(&mut self) {
		let _ = self.child.start_kill();
	}
}

async fn spawn_node(port: u16, bootstrap: bool, node_id: u64) -> Result<NodeHandle> {
	let data_dir = TempDir::new()?;
	let config_path = data_dir.path().join("config.toml");

	let config_contents = format!(
		"[server]\nhost = \"127.0.0.1\"\nport = {}\n\n[storage]\nengine = \"aidb\"\ndata_dir = \"{}\"\ndatabases = 4\n\n[logging]\nlevel = \"warn\"\n",
		port,
		data_dir.path().display()
	);

	fs::write(&config_path, config_contents)?;

	let bin = std::env::var("CARGO_BIN_EXE_aikv").unwrap_or_else(|_| {
		// Fallback: assume the binary is in target/debug/aikv (for debug builds)
		// or target/release/aikv (for release builds)
		let debug_path = "target/debug/aikv";
		let release_path = "target/release/aikv";
		if std::path::Path::new(debug_path).exists() {
			debug_path.to_string()
		} else if std::path::Path::new(release_path).exists() {
			release_path.to_string()
		} else {
			panic!("Could not find aikv binary in target/debug/aikv or target/release/aikv");
		}
	});

	println!("Using binary: {}", bin);

	let mut cmd = Command::new(bin);
	cmd.env("AIKV_BOOTSTRAP", if bootstrap { "1" } else { "0" })
		.env("AIKV_NODE_ID", format!("{:x}", node_id))
		.arg("--config")
		.arg(&config_path)
		.stdout(std::process::Stdio::inherit())
		.stderr(std::process::Stdio::inherit());

	let child = cmd.spawn()?;

	// Allow the server to start listening (MetaRaft bootstrap can take a bit).
	// Borrow the mutable child before moving it into the handle so we can observe early exit.
	let mut child_for_wait = child;
	wait_for_ready(&mut child_for_wait, port, Duration::from_secs(30)).await?;

	// Move the child back after readiness check.
	let child = child_for_wait;

	Ok(NodeHandle {
		child,
		data_dir,
		port,
	})
}

async fn wait_for_ready(child: &mut Child, port: u16, timeout: Duration) -> Result<()> {
	let start = Instant::now();
	loop {
		// If the child process exited early, surface that immediately.
		if let Some(status) = child.try_wait()? {
			bail!(
				"Server on port {} exited early with status {:?} before becoming ready",
				port,
				status
			);
		}

		let client = Client::open(format!("redis://127.0.0.1:{}/", port));
		if let Ok(mut con) = client.and_then(|c| c.get_connection()) {
			if redis::cmd("PING").query::<String>(&mut con).is_ok() {
				return Ok(());
			}
		}

		if start.elapsed() > timeout {
			bail!("Server on port {} not ready within {:?}", port, timeout);
		}
		sleep(Duration::from_millis(200)).await;
	}
}

async fn wait_for_cluster_nodes(port: u16, expected_nodes: usize, timeout: Duration) -> Result<()> {
	let client = Client::open(format!("redis://127.0.0.1:{}/", port))?;
	let start = Instant::now();

	loop {
		if let Ok(mut con) = client.get_connection() {
			if let Ok(raw) = redis::cmd("CLUSTER").arg("NODES").query::<String>(&mut con) {
				let nodes: Vec<&str> = raw
					.lines()
					.filter(|l| !l.trim().is_empty())
					.collect();
				if nodes.len() >= expected_nodes {
					return Ok(());
				}
			}
		}

		if start.elapsed() > timeout {
			bail!(
				"Cluster on port {} did not reach {} nodes within {:?}",
				port,
				expected_nodes,
				timeout
			);
		}

		sleep(Duration::from_millis(300)).await;
	}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metaraft_converges_across_processes() -> Result<()> {
	println!("Starting bootstrap node on port 6380...");
	// Start bootstrap node
	let node1 = spawn_node(6380, true, 0x1).await?;
	println!("Bootstrap node started");

	println!("Starting follower node on port 6381...");
	// Start follower node
	let node2 = spawn_node(6381, false, 0x2).await?;
	println!("Follower node started");

	println!("Connecting to bootstrap node...");
	let mut con1 = Client::open("redis://127.0.0.1:6380/")?.get_connection()?;
	println!("Connected to bootstrap node");

	// Verify CLUSTER NODES works before MEET
	println!("Checking CLUSTER NODES before MEET...");
	let before_meet: String = redis::cmd("CLUSTER").arg("NODES").query(&mut con1)?;
	println!("CLUSTER NODES before MEET:\n{}", before_meet);

	println!("Sending CLUSTER MEET command...");
	// Initiate cluster meet from node1 to node2
	let result: String = redis::cmd("CLUSTER")
		.arg("MEET")
		.arg("127.0.0.1")
		.arg("6381")
		.query(&mut con1)?;
	println!("CLUSTER MEET result: {}", result);

	// Verify CLUSTER NODES after MEET
	println!("Checking CLUSTER NODES after MEET...");
	let after_meet: String = redis::cmd("CLUSTER").arg("NODES").query(&mut con1)?;
	println!("CLUSTER NODES after MEET:\n{}", after_meet);

	println!("Waiting for cluster nodes to converge...");
	// Wait for both nodes to see the cluster view converge via MetaRaft
	wait_for_cluster_nodes(6380, 2, Duration::from_secs(20)).await?;
	wait_for_cluster_nodes(6381, 2, Duration::from_secs(20)).await?;
	println!("Cluster converged successfully!");

	println!("Cleaning up nodes...");
	node1.kill().await;
	node2.kill().await;
	println!("Test completed successfully!");

	Ok(())
}
