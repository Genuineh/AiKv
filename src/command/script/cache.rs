//! SCRIPT LOAD 脚本缓存 (LRU 256; EVAL 不写入)

use std::collections::{HashMap, VecDeque};
use std::sync::RwLock;

const MAX_SCRIPTS: usize = 256;

#[derive(Clone, Debug)]
pub struct CachedScript {
  pub script: String,
}

pub struct ScriptCache {
  inner: RwLock<ScriptCacheInner>,
}

struct ScriptCacheInner {
  map: HashMap<String, CachedScript>,
  order: VecDeque<String>,
}

impl Default for ScriptCache {
  fn default() -> Self {
    Self {
      inner: RwLock::new(ScriptCacheInner {
        map: HashMap::new(),
        order: VecDeque::new(),
      }),
    }
  }
}

impl ScriptCache {
  pub fn get(&self, sha1: &str) -> Option<String> {
    let mut inner = self.inner.write().ok()?;
    if !inner.map.contains_key(sha1) {
      return None;
    }
    inner.order.retain(|s| s != sha1);
    inner.order.push_back(sha1.to_string());
    inner.map.get(sha1).map(|c| c.script.clone())
  }

  pub fn insert(&self, sha1: String, script: String) {
    let Ok(mut inner) = self.inner.write() else {
      return;
    };
    if inner.map.contains_key(&sha1) {
      inner.order.retain(|s| s != &sha1);
    } else if inner.map.len() >= MAX_SCRIPTS {
      if let Some(old) = inner.order.pop_front() {
        inner.map.remove(&old);
      }
    }
    inner.order.push_back(sha1.clone());
    inner.map.insert(sha1, CachedScript { script });
  }

  pub fn exists(&self, sha1: &str) -> bool {
    self
      .inner
      .read()
      .map(|i| i.map.contains_key(sha1))
      .unwrap_or(false)
  }

  pub fn flush(&self) {
    if let Ok(mut inner) = self.inner.write() {
      inner.map.clear();
      inner.order.clear();
    }
  }
}
