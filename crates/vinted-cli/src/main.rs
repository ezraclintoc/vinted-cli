use vinted_core::VintedClient;

#[tokio::main]
async fn main() {
    // Scaffold placeholder until the CLI is implemented.
    let client = VintedClient::new("https://www.vinted.com").unwrap();
    drop(client);
    println!("vinted-cli scaffold");
}
