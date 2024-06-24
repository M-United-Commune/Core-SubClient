use std::{io::Write, process::Stdio, sync::Arc, vec};

use futures_util::{SinkExt, StreamExt};
use lib::config::SubConfig;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
mod lib;
#[tokio::main]
async fn main() {
    // 服务端配置
    let config = lib::config::SubConfig::default();

    // 创建文件夹：server、plugins、config、worlds
    let filedirs = vec!["server", "plugins", "config", "worlds"];
    for filedir in filedirs {
        std::fs::create_dir_all(format!("./{}", filedir)).expect("创建文件夹失败");
    }

    // 服务端状态
    let mut substate = Arc::new(Mutex::new(SubState::Stopped));
    let mut subchild = Arc::new(Mutex::new(None));
    let server_name_encoded =
        utf8_percent_encode(&config.server_name, NON_ALPHANUMERIC).to_string();
    let url = format!("{}?server_name={}", config.uri, server_name_encoded);

    let (ws_stream, _) = connect_async(&url).await.expect("Failed to connect");
    // 美化输出
    println!("Server-Name: {}", config.server_name);
    println!("Server-URI: {}", config.uri);
    println!("Server-Jar: {}", config.server_jar);

    // 接受消息
    let (mut _write, mut read) = ws_stream.split();

    // 异步收取消息
    while let Some(msg) = read.next().await {
        let msg = msg.unwrap();
        let message = match msg {
            Message::Text(text) => text,
            _ => continue,
        };
        let messagedata: MessageData = serde_json::from_str(&message).unwrap();
        let _messsage_event =
            handle_message(&mut substate, &mut subchild, config.clone(), &messagedata).await;
        let result = ResultMessage {
            server: messagedata.server.clone(),
            sub_state: substate.lock().await.clone(),
        };
        let result = serde_json::to_string(&result).unwrap();
        _write.send(Message::Text(result)).await.unwrap();
    }
}

// 处理消息
async fn handle_message(
    substate: &mut Arc<Mutex<SubState>>,
    subchild: &mut Arc<Mutex<Option<std::process::Child>>>,
    config: SubConfig,
    message: &MessageData,
) -> ClientAction {
    match message.action {
        ClientAction::Start => {
            if substate.clone().lock().await.clone() == SubState::Stopped {
                let mut substate = substate.lock().await;
                *substate = SubState::Starting;
                println!("正在启动服务器");
                {
                    let server_jar = config.server_jar.clone();
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
            } else {
                println!("服务器已经启动");
            }
            ClientAction::Start
        }
        ClientAction::Stop => {
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
                } else {
                    println!("服务器关闭失败");
                }
            }
            ClientAction::Stop
        }
        ClientAction::Restart => {
            println!("Restarting server: {}", message.server);
            ClientAction::Restart
        }
    }
}
pub enum MessageEvent {
    Message(Message),
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct MessageData {
    pub server: String,
    pub action: ClientAction,
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
}

impl From<String> for ClientAction {
    fn from(value: String) -> Self {
        match value.as_str() {
            "start" => ClientAction::Start,
            "stop" => ClientAction::Stop,
            "restart" => ClientAction::Restart,
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
    // 切换到路径server
    std::env::set_current_dir("server").expect("切换路径失败");
    Command::new("java")
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
