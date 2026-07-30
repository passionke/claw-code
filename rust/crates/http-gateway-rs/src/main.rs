//! http-gateway-rs binary entry. Author: kejiqing

#[tokio::main]
async fn main() {
    http_gateway_rs::bootstrap::run().await;
}
