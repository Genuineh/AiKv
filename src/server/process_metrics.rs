//! 读取当前进程 /proc 指标 (Linux).

/// 时钟 tick 频率 (Linux 上通常为 100).
#[cfg(target_os = "linux")]
const USER_HZ: f64 = 100.0;

pub fn read_resident_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(kb) = line.strip_prefix("VmRSS:") {
                let kb: u64 = kb.split_whitespace().next()?.parse().ok()?;
                return Some(kb * 1024);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// 系统物理内存总量 (MemTotal), 单位字节.
pub fn read_total_system_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        for line in meminfo.lines() {
            if let Some(kb) = line.strip_prefix("MemTotal:") {
                let kb: u64 = kb.split_whitespace().next()?.parse().ok()?;
                return Some(kb * 1024);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// 当前进程可执行文件路径.
pub fn read_executable_path() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link("/proc/self/exe")
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }
}

/// 累计 CPU 时间 user/system, 单位秒.
pub fn read_cpu_user_sys_seconds() -> Option<(f64, f64)> {
    #[cfg(target_os = "linux")]
    {
        read_cpu_jiffies().map(|(utime, stime)| (utime as f64 / USER_HZ, stime as f64 / USER_HZ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// 累计 CPU 时间 (user + system), 单位秒.
#[cfg(feature = "monitoring")]
pub fn read_cpu_seconds() -> Option<f64> {
    read_cpu_user_sys_seconds().map(|(user, sys)| user + sys)
}

#[cfg(target_os = "linux")]
fn read_cpu_jiffies() -> Option<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rparen = stat.rfind(')')?;
    let fields: Vec<&str> = stat[rparen + 2..].split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some((utime, stime))
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn read_cpu_jiffies() -> Option<(u64, u64)> {
    None
}

/// 累计磁盘读写字节 (read_bytes, write_bytes).
#[cfg(feature = "monitoring")]
pub fn read_io_bytes() -> Option<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        let io = std::fs::read_to_string("/proc/self/io").ok()?;
        let mut read_bytes = None;
        let mut write_bytes = None;
        for line in io.lines() {
            if let Some(v) = line.strip_prefix("read_bytes:") {
                read_bytes = v.trim().parse().ok();
            } else if let Some(v) = line.strip_prefix("write_bytes:") {
                write_bytes = v.trim().parse().ok();
            }
        }
        Some((read_bytes?, write_bytes?))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}
