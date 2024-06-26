// 配置文件

use std::{
    fs::File,
    io::{Read, Write},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SubConfig {
    // 服务器配置
    pub uri: String,
    pub server_name: String,
    pub server_jar: String,
}

impl Default for SubConfig {
    fn default() -> Self {
        let file_path = "config.yml";
        let config = SubConfig {
            uri: "ws://127.0.0.1:2024/api/subserver".to_string(), // 服务器地址
            server_name: "服务器名称".to_string(),                // 服务器名称
            server_jar: "paper-1.20.6-147.jar".to_string(),       // 服务器jar包
        };
        match read_yml(&file_path) {
            Ok(config) => config,
            Err(_err) => {
                // 当前路径
                let _ = write_config_to_yml(&config, file_path);
                config
            }
        }
    }
}
impl SubConfig {
    pub fn write_to_file(&self) -> Result<(), Box<dyn std::error::Error>> {
        let file_path = "config.yml";
        write_config_to_yml(self, file_path)
    }
}

// 写入到yml文件
pub fn write_config_to_yml(
    config: &SubConfig,
    file_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let yaml_string = serde_yaml::to_string(config)?;
    let mut file = File::create(file_path)?;
    file.write_all(yaml_string.as_bytes())?;
    Ok(())
}

pub fn read_yml(file_path: &str) -> Result<SubConfig, Box<dyn std::error::Error>> {
    let mut file = File::open(file_path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let config: SubConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}
