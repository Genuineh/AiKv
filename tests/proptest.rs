//! aikv property-based 测试
//!
//! ```bash
//! PROPTEST_CASES=100 cargo test --features cluster --test proptest -- --test-threads=1
//! ```

#[path = "proptest/resp.rs"]
mod resp;

#[path = "proptest/ttl.rs"]
mod ttl;
