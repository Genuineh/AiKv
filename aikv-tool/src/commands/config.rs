//! ak config 命令 - 管理工具配置
//!
//! 子命令: 
//! - `ak config get`  查看当前生效的配置
//! - `ak config set`  设置配置项
//! - `ak config sync` 同步配置到当前 schema
//! - `ak config path` 显示配置文件路径

use comfy_table::presets::UTF8_FULL;
use comfy_table::Table;

use crate::paths;
use crate::resources::config::AkConfig;
use crate::{ConfigAction, ConfigArgs, OutputFormat};

pub async fn execute(args: ConfigArgs, config: AkConfig) -> anyhow::Result<()> {
    match args.action {
        ConfigAction::Get(get_args) => {
            match get_args.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&config)?);
                }
                OutputFormat::Yaml => {
                    println!("{}", serde_yaml::to_string(&config)?);
                }
                OutputFormat::Table => {
                    let mut table = Table::new();
                    table
                        .load_preset(UTF8_FULL)
                        .set_header(vec!["Property", "Value"]);

                    table.add_row(vec![
                        "Schema Version",
                        &config.schema_version.to_string(),
                    ]);
                    table.add_row(vec![
                        "Project Root",
                        &format!("{:?}", config.project.root),
                    ]);
                    table.add_row(vec!["Mode", &config.defaults.mode.to_string()]);
                    table.add_row(vec!["Topo", &config.defaults.topo.to_string()]);
                    table.add_row(vec!["Port", &config.defaults.port.to_string()]);
                    table.add_row(vec!["Docker Image", &config.defaults.docker_image]);

                    if let Some(ref source) = config.source {
                        table.add_row(vec!["Config Source", &format!("{:?}", source)]);
                    }

                    println!("\nAiKv config:");
                    println!("{table}");
                }
            }
        }

        ConfigAction::Set(set_args) => {
            let mut config = config;
            let parts: Vec<&str> = set_args.value.splitn(2, '=').collect();
            if parts.len() != 2 {
                anyhow::bail!(
                    "invalid format: use key=value (e.g. ak config set project.root=/path/to/aikv)"
                );
            }

            let key = parts[0].trim();
            let value = parts[1].trim();

            let result = config.set_field(key, value)?;
            println!("{}", result);

            config.save()?;
        }

        ConfigAction::Sync => {
            config.save()?;
        }

        ConfigAction::Path => {
            if let Some(ref source) = config.source {
                println!("{}", source.display());
            } else if let Some(path) = paths::config_path() {
                println!("{} (not created)", path.display());
            } else {
                println!("could not determine config path");
            }
        }
    }

    Ok(())
}
