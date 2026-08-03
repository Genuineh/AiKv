//! 连接级集群状态: 每个 TCP 连接独立持有 ASKING / READONLY 标志, 供路由决策使用.
//! ASKING 一次性 (每命令执行后由 Connection 调用 `reset_asking`); READONLY 使副本读放行.

/// 连接级集群状态 (每个 TCP 连接独立持有).
#[derive(Debug, Clone)]
pub struct ClusterConnectionState {
    asking: bool,
    readonly: bool,
}

impl Default for ClusterConnectionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ClusterConnectionState {
    pub fn new() -> Self {
        Self {
            asking: false,
            readonly: false,
        }
    }

    pub fn is_asking(&self) -> bool {
        self.asking
    }
    pub fn set_asking(&mut self, v: bool) {
        self.asking = v;
    }
    pub fn reset_asking(&mut self) {
        self.asking = false;
    }

    pub fn is_readonly(&self) -> bool {
        self.readonly
    }
    pub fn set_readonly(&mut self, v: bool) {
        self.readonly = v;
    }
}
