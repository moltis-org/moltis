fn main() {
    let sdl = include_str!(concat!(env!("OUT_DIR"), "/schema.graphqls"));

    let path = std::env::args().nth(1);
    match path {
        Some(p) => {
            if let Some(parent) = std::path::Path::new(&p).parent() {
                std::fs::create_dir_all(parent).expect("failed to create parent directories");
            }
            std::fs::write(&p, sdl).expect("failed to write schema file");
            eprintln!("Wrote GraphQL schema to {p}");
        },
        None => print!("{sdl}"),
    }
}
