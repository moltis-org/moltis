//! Integration tests for the moltis-graphql crate.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "integration/common.rs"]
mod common;
#[path = "integration/mutation_tests.rs"]
mod mutation_tests;
#[path = "integration/query_tests.rs"]
mod query_tests;
#[path = "integration/schema_tests.rs"]
mod schema_tests;
#[path = "integration/subscription_tests.rs"]
mod subscription_tests;
