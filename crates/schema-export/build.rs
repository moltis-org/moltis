use std::{env, fs, path::PathBuf, sync::Arc};

fn main() {
    println!("cargo::rerun-if-changed=../graphql/src/");

    let services = Arc::new(moltis_service_traits::Services::default());
    let (tx, _rx) = tokio::sync::broadcast::channel(1);
    let schema = moltis_graphql::build_schema(services, tx);
    let sdl = schema.sdl();

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    fs::write(out_dir.join("schema.graphqls"), sdl).expect("failed to write schema.graphqls");
}
