/// AiKv 集群模式示例 (需要 --features cluster).
///
/// 演示 Redis Cluster 兼容的 CRC16 槽位计算和 hash tag 用法.
///
/// 运行: cargo run --features cluster --example cluster
fn main() -> Result<(), Box<dyn std::error::Error>> {
  println!("=== AiKv 集群示例 ===\n");

  #[cfg(feature = "cluster")]
  {
    // 1. CRC16 槽位计算
    let keys = ["mykey", "foo", "bar", "user:1001", "counter"];
    println!("  槽位计算:");
    for key in &keys {
      let slot = aikv::cluster::key_to_slot(key.as_bytes());
      println!("    CLUSTER KEYSLOT {:<16} -> slot {}", key, slot);
    }

    // 2. Hash tag 测试
    println!("\n  Hash tag 提取:");
    let tagged_keys = [
      ("{user:1001}.name", "user:1001"),
      ("{user:1001}.email", "user:1001"),
      ("{shopping_cart}:123", "shopping_cart"),
      ("plainkey", "(no tag)"),
    ];
    for (key, _expected_tag) in &tagged_keys {
      let slot = aikv::cluster::key_to_slot(key.as_bytes());
      let tag = aikv::cluster::extract_hash_tag(key.as_bytes());
      let tag_str = if tag == key.as_bytes() {
        "(none)".to_string()
      } else {
        String::from_utf8_lossy(tag).to_string()
      };
      println!("    {:<30} -> slot {}, hash_tag={}", key, slot, tag_str);
    }

    // 3. 验证 hash tag 确保相关 key 在同一 slot
    println!("\n  Hash tag 保证同 slot:");
    let slot1 = aikv::cluster::key_to_slot(b"{user:1001}.name");
    let slot2 = aikv::cluster::key_to_slot(b"{user:1001}.email");
    assert_eq!(slot1, slot2, "hash tag 应保证相同 slot");
    println!("    {{user:1001}}.name  -> slot {}", slot1);
    println!("    {{user:1001}}.email -> slot {}", slot2);
    println!("    相同 hash tag -> 相同 slot ✅");
  }

  #[cfg(not(feature = "cluster"))]
  {
    println!("  ℹ️  请使用 --features cluster 构建此示例:");
    println!("  cargo run --features cluster --example cluster");
  }

  println!("\n=== 示例完成 ===");
  Ok(())
}
