//! `ak build` - build binary or Docker image

use colored::*;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::resources::config::AkConfig;
use crate::runtime::docker;
use crate::{BuildArgs, RunMode};

pub async fn execute(
    args: BuildArgs,
    mode: RunMode,
    is_cluster: bool,
    config: &AkConfig,
    project_root: &Path,
) -> anyhow::Result<()> {

    match mode {
        RunMode::Bin => {
            if args.image.is_some() {
                anyhow::bail!(
                    "option `-i/--image` is only valid for Docker mode (current: bin)\n  \
                     help: use `-m docker` or set `defaults.mode = \"docker\"` in config"
                );
            }

            // -f: force rebuild (cargo clean then build)
            if args.force {
                println!("Force rebuild: cleaning target then building...");
                let clean_status = Command::new("cargo")
                    .current_dir(project_root)
                    .arg("clean")
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .status()
                    .await?;
                if !clean_status.success() {
                    anyhow::bail!("cargo clean failed");
                }
            }

            // Check Cargo is available
            let cargo_check = Command::new("cargo")
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            
            if cargo_check.is_err() || !cargo_check.unwrap().success() {
                anyhow::bail!("Rust toolchain (cargo) not found. Install from https://rustup.rs/");
            }

            println!("Building AiKv binary...");
            println!("   {} {:?}", "Project root:".blue(), project_root);

            let mut cmd = Command::new("cargo");
            cmd.current_dir(project_root);
            cmd.arg("build");

            if args.release {
                cmd.arg("--release");
                println!("   {} {}", "Build profile:".blue(), "Release".yellow());
            } else {
                println!("   {} {}", "Build profile:".blue(), "Debug".cyan());
            }

            if is_cluster {
                cmd.arg("--features").arg("cluster");
                println!("   {} {}", "Feature set:".blue(), "Cluster (Enabled)".magenta());
            } else {
                println!("   {} {}", "Feature set:".blue(), "Single Node".white());
            }

            println!("\n{}\n", "--- Cargo Build Output ---".bright_black());

            let status = cmd
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await?;

            if status.success() {
                let mode_dir = if args.release { "release" } else { "debug" };
                let abs_bin_path = project_root.join("target").join(mode_dir).join("aikv");
                println!(
                    "\n{}\n   {} {:?}",
                    "Build succeeded! ".green().bold(),
                    "Binary path:".bright_black(),
                    abs_bin_path
                );
            } else {
                println!("\n{}", "Build failed, check the output above. ".red().bold());
                anyhow::bail!("Cargo build failed");
            }
        }
        RunMode::Docker => {
            docker::check_docker_alive().await?;

            // Use defaults.docker_image when -i/--image is not set
            let image_name = args
                .image
                .clone()
                .unwrap_or_else(|| config.defaults.docker_image.clone());

            if !args.force && docker::image_exists(&image_name).await {
                println!(
                    "{} image {} already exists. ",
                    "Skipping build:".yellow().bold(),
                    image_name.cyan()
                );
                println!(
                    "   To overwrite, pass {}.",
                    "--force".bold()
                );
                return Ok(());
            }

            println!("Building AiKv Docker image...");

            println!("   {} {:?}", "Project root:".blue(), project_root);
            println!("   {} {}", "Image tag:".blue(), image_name.yellow());

            let mut cmd = Command::new("docker");
            cmd.current_dir(project_root);
            cmd.arg("build");
            cmd.arg("-t").arg(&image_name);

            // Pass cluster feature to Docker build ARG when enabled
            if is_cluster {
                cmd.arg("--build-arg").arg("FEATURES=cluster");
                println!("   {} {}", "Feature set:".blue(), "Cluster (Enabled)".magenta());
            }

            // Dockerfile path
            cmd.arg(".");

            println!("\n{}\n", "--- Docker Build Output ---".bright_black());

            let status = cmd
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await?;

            if status.success() {
                println!(
                    "\n{}\n   {} {}",
                    "Image build succeeded! ".green().bold(),
                    "Run the container:".bright_black(),
                    format!("docker run -d -p 6379:6379 {}", image_name).cyan()
                );
            } else {
                println!("\n{}", "Image build failed, ensure Docker daemon is running. ".red().bold());
                anyhow::bail!("Docker build failed");
            }
        }
    }
    Ok(())
}
