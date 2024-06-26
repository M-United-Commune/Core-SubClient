use std::borrow::BorrowMut;
use std::sync::Arc;

use reqwest::Client;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::config::SubConfig;

// 下载核心
pub async fn download_core(config: &mut Arc<Mutex<SubConfig>>) {
    let config = config.lock().await;
    // 创建一个 reqwest 的客户端
    let client = Client::new();
    // 将开头的ws替换为http
    let uri = config.uri.replace("ws", "http");
    // 去掉末尾的.jar
    let server_jar = config.server_jar.replace(".jar", "");
    // http://127.0.0.1:2024/api/subserver/down_server_jar?file_name=paper-1.20.6-147
    let url = format!("{}/down_server_jar?file_name={}", uri, server_jar);
    println!("正在下载服务器核心: {}", url);
    let mut response = client.get(&url).send().await.unwrap();
    let mut file = File::create(format!("server/{}.jar", server_jar))
        .await
        .unwrap();
    while let Some(mut chunk) = response.chunk().await.unwrap() {
        file.write_all(chunk.borrow_mut()).await.unwrap();
    }
    println!("服务器核心下载完成");
}
