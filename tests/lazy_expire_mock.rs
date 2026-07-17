//! @component aikv-storage
//! 懒过期 mock 回归: 清理 delete 失败或禁止时, 过期读仍须 Ok(None).
#[path = "modules/storage/lazy_expire_mock.rs"]
mod lazy_expire_mock;
