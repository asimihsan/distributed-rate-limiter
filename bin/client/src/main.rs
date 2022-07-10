#[tokio::main]
async fn main() {
    println!("Hello, world!");
    network::client().await.unwrap()
}
