//! @component aikv-cluster
//! 集群副本懒过期回归: 过期 key 读应返回 nil, 不得因清理 delete 失败变成集群错误.
#[cfg(feature = "cluster")]
#[path = "modules/cluster/lazy_expire.rs"]
mod tests;
