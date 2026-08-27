//! OTel counter / updown delta 与 cluster redirect 属性解析.

pub(super) fn counter_delta(current: u64, last: &mut u64) -> u64 {
    let delta = current.saturating_sub(*last);
    *last = current;
    delta
}

#[cfg(feature = "cluster")]
pub(super) fn cluster_redirect_type_from_cmd(cmd: &str) -> Option<String> {
    const PREFIX: &str = "CLUSTER.redirect.";
    if cmd.len() <= PREFIX.len() || !cmd[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    Some(cmd[PREFIX.len()..].to_ascii_lowercase())
}

pub(super) fn updown_delta(current: i64, last: &mut i64) -> i64 {
    let delta = current - *last;
    *last = current;
    delta
}

#[cfg(all(test, feature = "cluster", feature = "monitoring"))]
mod cluster_redirect_tests {
    use super::cluster_redirect_type_from_cmd;

    #[test]
    fn cluster_redirect_type_matches_metrics_key_casing() {
        assert_eq!(
            cluster_redirect_type_from_cmd("CLUSTER.redirect.moved").as_deref(),
            Some("moved")
        );
        assert_eq!(
            cluster_redirect_type_from_cmd("CLUSTER.REDIRECT.ASK").as_deref(),
            Some("ask")
        );
        assert!(cluster_redirect_type_from_cmd("GET").is_none());
    }
}
