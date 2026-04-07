use custom_utils::logger;
use log::LevelFilter::Info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = logger::logger_feature("app", "debug", Info, false).build();
    // remember to print msg via stdio
    rust_template::run().await
}
