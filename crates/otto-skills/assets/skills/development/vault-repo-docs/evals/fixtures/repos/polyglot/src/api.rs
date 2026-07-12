#[post("/orders")]
async fn create_order() {}

fn main() { start_server(); graceful_shutdown(); }
