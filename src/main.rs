use std::{env, io::Write, process::Stdio, sync::Arc, vec};

use futures_util::{SinkExt, StreamExt};
use lib::config::SubConfig;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

mod lib;
#[tokio::main]
async fn main() {
    // 服务端配置
    let mut config = Arc::new(Mutex::new(lib::config::SubConfig::default()));

    // 创建文件夹：server、plugins、config、worlds
    let filedirs = vec!["server", "plugins", "config", "worlds"];
    for filedir in filedirs {
        std::fs::create_dir_all(format!("./{}", filedir)).expect("创建文件夹失败");
    }

    // 服务端状态
    let mut substate = Arc::new(Mutex::new(SubState::Stopped));
    // 服务端进程
    let mut subchild = Arc::new(Mutex::new(None));

    let server_name_encoded =
        utf8_percent_encode(&config.lock().await.server_name, NON_ALPHANUMERIC).to_string();
    let url = format!(
        "{}/ws?server_name={}",
        config.lock().await.uri,
        server_name_encoded
    );

    loop {
        let main_ws_stream = match connect_async(&url).await {
            Ok((ws_stream, _)) => {
                println!("连接成功");
                ws_stream
            }
            Err(e) => {
                println!("连接失败: {:?}", e);
                continue;
            }
        };

        // 美化输出
        println!("Server-Name: {}", config.lock().await.server_name);
        println!("Server-URI: {}", config.lock().await.uri);
        println!("Server-Jar: {}", config.lock().await.server_jar);
        // 接受消息
        let (mut ws_write, mut read) = main_ws_stream.split();

        // 异步收取消息
        while let Some(ws_msg_datas) = read.next().await {
            let (client_action, messagedata) = match ws_msg_datas {
                Ok(message) => match message {
                    Message::Binary(bin) => {
                        // 解析消息为指令
                        let client_action: ClientAction = bin[0].into();
                        let messagedata = bin[1..].to_vec();
                        (client_action, messagedata)
                    }
                    _ => continue,
                },
                Err(e) => {
                    println!("接受消息失败: {:?}", e);
                    continue;
                }
            };

            match client_action {
                ClientAction::Start => {
                    start_server(&mut substate, &mut subchild, &mut config).await;
                }
                ClientAction::Stop => {
                    stop_server(&mut substate, &mut subchild).await;
                }
                ClientAction::Restart => {
                    let _substate = stop_server(&mut substate, &mut subchild).await;
                    if _substate == SubState::Stopped {
                        start_server(&mut substate, &mut subchild, &mut config).await;
                    }
                }
                ClientAction::AcceptCore => {
                    if substate.lock().await.clone() == SubState::Running {
                        stop_server(&mut substate, &mut subchild).await;
                    }
                    let file_name = String::from_utf8(messagedata).unwrap();
                    config.lock().await.server_jar = file_name + ".jar";
                    config.lock().await.write_to_file().err();
                    lib::download::download_core(&mut config).await;
                }
                ClientAction::GetState => {
                    let result = ResultMessage {
                        server: config.lock().await.server_name.clone(),
                        sub_state: substate.clone().lock().await.clone(),
                    };
                    ws_write
                        .send(Message::Text(serde_json::to_string(&result).unwrap()))
                        .await
                        .expect("发送消息失败");
                }
                ClientAction::AcceptPlugin => todo!(),
                ClientAction::AcceptConfig => todo!(),
                ClientAction::AcceptWorld => todo!(),
            }
        }
    }
}

//  启动子服
async fn start_server(
    substate: &mut Arc<Mutex<SubState>>,
    subchild: &mut Arc<Mutex<Option<std::process::Child>>>,
    config: &mut Arc<Mutex<SubConfig>>,
) -> SubState {
    if substate.clone().lock().await.clone() == SubState::Stopped {
        let mut substate = substate.lock().await;
        *substate = SubState::Starting;
        println!("正在启动服务器");
        {
            let server_jar = config.lock().await.clone().server_jar;
            let child = start_client(server_jar).await;
            match child {
                Ok(child) => {
                    let mut subchild = subchild.lock().await;
                    *subchild = Some(child);
                }
                Err(e) => {
                    println!("服务器启动失败: {:?}", e);
                }
            }
        }
        *substate = SubState::Running;
        println!("服务器启动成功");
    }
    substate.clone().lock().await.clone()
}

// 关闭子服
async fn stop_server(
    substate: &mut Arc<Mutex<SubState>>,
    subchild: &mut Arc<Mutex<Option<std::process::Child>>>,
) -> SubState {
    if substate.clone().lock().await.clone() == SubState::Running {
        let mut substate = substate.lock().await;
        if let Some(mut child) = subchild.lock().await.take() {
            println!("正在关闭服务器");
            *substate = SubState::Stopping;
            // 尝试使用指令关闭服务器
            match child.stdin.take() {
                Some(mut stdin) => {
                    // 向服务器发送关闭指令
                    if let Err(e) = stdin.write_all(b"stop\n") {
                        println!("服务器关闭失败: {:?}", e);
                        child.kill().expect("对服务器kill关闭失败");
                    }
                }
                None => {
                    println!("服务器关闭失败");
                    child.kill().expect("服务器关闭失败");
                }
            }
            println!("服务器关闭成功");
            *substate = SubState::Stopped;
        }
    }
    substate.clone().lock().await.clone()
}

pub enum MessageEvent {
    Message(Message),
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct MessageData {
    pub action: ClientAction,
    pub path: Option<String>,
    pub data: Option<Vec<u8>>,
}

// 客户端操作
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub enum ClientAction {
    // 启动子服
    Start,
    // 关闭子服
    Stop,
    // 重启子服
    Restart,
    // 接受核心
    AcceptCore,
    // 接受插件
    AcceptPlugin,
    // 接受配置
    AcceptConfig,
    // 接受世界
    AcceptWorld,
    // 获取状态
    GetState,
}

impl From<String> for ClientAction {
    fn from(value: String) -> Self {
        match value.as_str() {
            "start" => ClientAction::Start,
            "stop" => ClientAction::Stop,
            "restart" => ClientAction::Restart,
            "accept_core" => ClientAction::AcceptCore,
            "accept_plugin" => ClientAction::AcceptPlugin,
            "accept_config" => ClientAction::AcceptConfig,
            "accept_world" => ClientAction::AcceptWorld,
            "get_state" => ClientAction::GetState,
            "Start" => ClientAction::Start,
            "Stop" => ClientAction::Stop,
            "Restart" => ClientAction::Restart,
            "AcceptCore" => ClientAction::AcceptCore,
            "AcceptPlugin" => ClientAction::AcceptPlugin,
            "AcceptConfig" => ClientAction::AcceptConfig,
            "AcceptWorld" => ClientAction::AcceptWorld,
            "GetState" => ClientAction::GetState,
            _ => panic!("Invalid value: {}", value),
        }
    }
}

impl From<u8> for ClientAction {
    fn from(value: u8) -> Self {
        match value {
            0 => ClientAction::Start,
            1 => ClientAction::Stop,
            2 => ClientAction::Restart,
            3 => ClientAction::AcceptCore,
            4 => ClientAction::AcceptPlugin,
            5 => ClientAction::AcceptConfig,
            6 => ClientAction::AcceptWorld,
            7 => ClientAction::GetState,
            _ => panic!("Invalid value: {}", value),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub enum SubState {
    // 子服未启动
    Stopped,
    // 子服启动中
    Starting,
    // 子服运行中
    Running,
    // 子服关闭中
    Stopping,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ResultMessage {
    server: String,
    sub_state: SubState,
}

impl From<String> for SubState {
    fn from(value: String) -> Self {
        match value.as_str() {
            "stopped" => SubState::Stopped,
            "starting" => SubState::Starting,
            "running" => SubState::Running,
            "stopping" => SubState::Stopping,
            _ => panic!("Invalid value: {}", value),
        }
    }
}

impl ToString for SubState {
    fn to_string(&self) -> String {
        match self {
            SubState::Stopped => "stopped".to_string(),
            SubState::Starting => "starting".to_string(),
            SubState::Running => "running".to_string(),
            SubState::Stopping => "stopping".to_string(),
        }
    }
}

use std::process::{Child, Command};
pub async fn start_client(server_jar: String) -> Result<Child, std::io::Error> {
    use systemstat::{Platform, System};
    let sys = System::new();
    // 计算可用内存
    let sys_info = match sys.memory() {
        Ok(mem) => {
            let memory_start = (mem.total.as_u64() as f64 * 0.1).round() as u64;
            let memory_max = (mem.total.as_u64() as f64 * 0.2).round() as u64;
            (memory_start, memory_max)
        }
        Err(x) => {
            println!("\n内存: 错误: {}", x);
            return Err(x);
        }
    };

    let java_memory_start: String = format!("{}", sys_info.0.to_string().replace(" ", "")); // 示例最小内存
    let java_memory_max: String = format!("{}", sys_info.1.to_string().replace(" ", "")); // 示例最大内存

    // java -Xms1024M -Xmx2048M -jar server.jar --nogui
    // 在指定路径下启动服务端

    Command::new("java")
        .current_dir("server")
        .args(&[
            "-Xms".to_string() + &java_memory_start,
            "-Xmx".to_string() + &java_memory_max,
            "-jar".to_string(),
            server_jar.to_string(),
            "--nogui".to_string(),
        ])
        .stdin(Stdio::piped())
        .spawn()
}
