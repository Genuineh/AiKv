use aidb::cluster::Router;

/// 集群客户端地址通告模式 (仅进程配置, 不写入 MetaRaft).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnounceMode {
  /// 使用 MetaRaft `client_addr` 中的 host:port.
  Fixed,
  /// `CLUSTER SLOTS` / MOVED 输出空 host, 客户端沿用种子连接地址 (Redis 7).
  UnknownEndpoint,
}

impl AnnounceMode {
  pub fn from_env() -> Self {
    match std::env::var("AIKV_CLUSTER_ANNOUNCE_MODE")
      .ok()
      .as_deref()
      .map(str::to_ascii_lowercase)
    {
      Some(ref s) if s == "fixed" => AnnounceMode::Fixed,
      Some(ref s) if s == "unknown" => AnnounceMode::UnknownEndpoint,
      None => AnnounceMode::UnknownEndpoint,
      Some(other) => {
        tracing::warn!(
          mode = %other,
          "unknown AIKV_CLUSTER_ANNOUNCE_MODE, using unknown"
        );
        AnnounceMode::UnknownEndpoint
      }
    }
  }
}

/// 将 Router 中的 client_addr 转换为客户端可见的 endpoint / MOVED 地址.
#[derive(Debug, Clone)]
pub struct AnnounceResolver {
  mode: AnnounceMode,
}

impl Default for AnnounceResolver {
  fn default() -> Self {
    Self {
      mode: AnnounceMode::UnknownEndpoint,
    }
  }
}

impl AnnounceResolver {
  pub fn new(mode: AnnounceMode) -> Self {
    Self { mode }
  }

  pub fn from_env() -> Self {
    Self::new(AnnounceMode::from_env())
  }

  pub fn mode(&self) -> AnnounceMode {
    self.mode
  }

  pub fn parse_raw_endpoint(addr_str: &str) -> Option<(String, u16)> {
    let (host, port_str) = addr_str.rsplit_once(':')?;
    let port: u16 = port_str.parse().ok()?;
    Some((host.to_string(), port))
  }

  pub fn endpoint_for_node(&self, node_id: u64, router: &Router) -> Option<(String, u16)> {
    let addr_str = router.get_node_addr(node_id)?;
    let (host, port) = Self::parse_raw_endpoint(&addr_str)?;
    Some(self.format_host_port(&host, port))
  }

  pub fn redirect_addr(&self, node_id: u64, router: &Router) -> Option<String> {
    let (host, port) = self.endpoint_for_node(node_id, router)?;
    Some(self.redirect_from_host_port(&host, port))
  }

  pub fn redirect_from_addr_str(&self, addr: &str) -> Option<String> {
    let (host, port) = Self::parse_raw_endpoint(addr)?;
    Some(self.redirect_from_host_port(&host, port))
  }

  /// 服务端跨分片转发用的 TCP 地址.
  ///
  /// 客户端 MOVED/ASK 在 `unknown` 模式下可能是 `:7379`, 进程内转发不能直连该字符串;
  /// 使用 MetaRaft `rpc_addr` 的主机名 + 客户端端口, 以便 Docker 网络内可达.
  pub fn tcp_connect_addr(
    &self,
    node_id: u64,
    redirect_addr: &str,
    meta: &aidb::cluster::meta_types::ClusterMeta,
  ) -> Option<String> {
    let (_, client_port) = Self::parse_raw_endpoint(redirect_addr)?;
    let node = meta.nodes.get(&node_id)?;
    let rpc_host = node.rpc_addr.rsplit_once(':')?.0;
    Some(format!("{rpc_host}:{client_port}"))
  }

  fn format_host_port(&self, host: &str, port: u16) -> (String, u16) {
    match self.mode {
      AnnounceMode::Fixed => (host.to_string(), port),
      AnnounceMode::UnknownEndpoint => (String::new(), port),
    }
  }

  fn redirect_from_host_port(&self, host: &str, port: u16) -> String {
    match self.mode {
      AnnounceMode::Fixed => format!("{host}:{port}"),
      AnnounceMode::UnknownEndpoint => format!(":{port}"),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::HashMap;
  use aidb::cluster::meta_types::{default_slot_table, SlotStatus};
  use aidb::cluster::Router;

  fn router_with_addr(node_id: u64, addr: &str) -> Router {
    let mut table = default_slot_table();
    table[0] = SlotStatus::Assigned(1);
    let mut group_nodes = HashMap::new();
    group_nodes.insert(1u64, vec![node_id]);
    let mut node_addrs = HashMap::new();
    node_addrs.insert(node_id, addr.to_string());
    Router::new(table, group_nodes, node_addrs)
  }

  #[test]
  fn fixed_mode_endpoint_and_redirect() {
    let resolver = AnnounceResolver::new(AnnounceMode::Fixed);
    let router = router_with_addr(1, "192.168.0.140:7379");
    assert_eq!(
      resolver.endpoint_for_node(1, &router),
      Some(("192.168.0.140".to_string(), 7379))
    );
    assert_eq!(
      resolver.redirect_addr(1, &router).as_deref(),
      Some("192.168.0.140:7379")
    );
  }

  #[test]
  fn unknown_mode_endpoint_and_redirect() {
    let resolver = AnnounceResolver::new(AnnounceMode::UnknownEndpoint);
    let router = router_with_addr(1, "192.168.0.140:7379");
    assert_eq!(
      resolver.endpoint_for_node(1, &router),
      Some((String::new(), 7379))
    );
    assert_eq!(resolver.redirect_addr(1, &router).as_deref(), Some(":7379"));
  }

  #[test]
  fn redirect_from_rpc_fallback_addr() {
    let resolver = AnnounceResolver::new(AnnounceMode::Fixed);
    assert_eq!(
      resolver
        .redirect_from_addr_str("aikv-4:17379")
        .as_deref(),
      Some("aikv-4:17379")
    );
    let unknown = AnnounceResolver::new(AnnounceMode::UnknownEndpoint);
    assert_eq!(
      unknown
        .redirect_from_addr_str("aikv-4:17379")
        .as_deref(),
      Some(":17379")
    );
  }

  #[test]
  fn tcp_connect_addr_uses_rpc_host_with_client_port() {
    use std::collections::HashMap;

    use aidb::cluster::meta_types::{ClusterMeta, NodeInfo, NodeRole, NodeStatus};

    let resolver = AnnounceResolver::new(AnnounceMode::UnknownEndpoint);
    let mut meta = ClusterMeta::default();
    meta.nodes.insert(
      4,
      NodeInfo {
        node_id: 4,
        rpc_addr: "aikv-4:17379".into(),
        client_addr: Some("127.0.0.1:7379".into()),
        role: NodeRole::Voter,
        status: NodeStatus::Online,
        registered_at: 0,
        tags: HashMap::new(),
      },
    );
    assert_eq!(
      resolver.tcp_connect_addr(4, ":7379", &meta).as_deref(),
      Some("aikv-4:7379")
    );
    assert_eq!(
      resolver
        .tcp_connect_addr(4, "127.0.0.1:7379", &meta)
        .as_deref(),
      Some("aikv-4:7379")
    );
  }
}
