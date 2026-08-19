//! @component aikv-cluster
// Cluster protocol L2 integration tests.
// CLUSTER_STATE_MGR is OnceLock — all tests use a shared setup via std::sync::Once.
#![cfg(feature = "cluster")]
#[path = "modules/cluster/integration.rs"]
mod tests;
