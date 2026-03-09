//! 通用辅助函数

use crate::constants::display;

/// 获取状态子目录名
pub fn get_state_subdir(is_cluster: bool) -> &'static str {
    use crate::constants::dirs;
    if is_cluster {
        dirs::CLUSTER
    } else {
        dirs::SINGLE
    }
}

/// 获取模式显示名称
pub fn get_mode_name(is_cluster: bool) -> &'static str {
    if is_cluster {
        display::MODE_CLUSTER
    } else {
        display::MODE_SINGLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::dirs;

    #[test]
    fn state_subdir_single() {
        assert_eq!(get_state_subdir(false), dirs::SINGLE);
        assert_eq!(get_state_subdir(false), "single");
    }

    #[test]
    fn state_subdir_cluster() {
        assert_eq!(get_state_subdir(true), dirs::CLUSTER);
        assert_eq!(get_state_subdir(true), "cluster");
    }

    #[test]
    fn mode_name_single() {
        assert_eq!(get_mode_name(false), display::MODE_SINGLE);
        assert_eq!(get_mode_name(false), "Single");
    }

    #[test]
    fn mode_name_cluster() {
        assert_eq!(get_mode_name(true), display::MODE_CLUSTER);
        assert_eq!(get_mode_name(true), "Cluster");
    }
}
