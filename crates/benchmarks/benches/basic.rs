fn main() {
    divan::main();
}

// #[divan::bench]
// fn parse_json() {
//     let json_str = r#"{"name":"moltis","version":"0.2.9","features":["agents","mcp","memory"]}"#;
//     divan::black_box(serde_json::from_str::<serde_json::Value>(json_str).unwrap());
// }
//
// #[divan::bench(args = [1, 2, 4, 8, 16, 32])]
// fn fibonacci(n: u64) -> u64 {
//     if n <= 1 {
//         1
//     } else {
//         fibonacci(n - 2) + fibonacci(n - 1)
//     }
// }
//
// #[divan::bench]
// fn string_operations() {
//     let text = "moltis is an AI-powered assistant platform";
//     let words: Vec<&str> = divan::black_box(text.split_whitespace().collect());
//     divan::black_box(words.len());
// }
//
// #[divan::bench(args = [10, 100, 1000])]
// fn vec_allocation(n: usize) {
//     let mut vec = Vec::with_capacity(n);
//     for i in 0..n {
//         vec.push(i);
//     }
//     divan::black_box(vec);
// }
