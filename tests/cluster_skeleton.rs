// AiKv cluster module skeleton test.
#[test]
#[cfg(feature = "cluster")]
fn cluster_module_exists() {
  let _ = aikv::cluster::state::CLUSTER_STATE_MGR.get();
}
