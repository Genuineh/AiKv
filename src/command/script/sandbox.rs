//! Lua 沙箱: StdLib 裁剪 + 危险全局函数封印
//!
//! mlua 创建 VM 时总会加载 `luaopen_base` (spec 中的 BASE); 此处再显式启用 TABLE/STRING/MATH/UTF8.

use mlua::{Lua, LuaOptions, StdLib, Value as LuaValue};

use crate::error::{Error, Result};

const DISABLED_GLOBALS: &[&str] = &[
  "load",
  "loadfile",
  "dofile",
  "require",
  "rawget",
  "rawset",
  "setmetatable",
  "getmetatable",
  "collectgarbage",
];

pub fn new_sandbox_lua() -> Result<Lua> {
  let lua = Lua::new_with(
    StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
    LuaOptions::default(),
  )
  .map_err(|e| Error::Command(format!("ERR failed to create Lua: {e}")))?;

  harden_globals(&lua)?;
  Ok(lua)
}

fn harden_globals(lua: &Lua) -> Result<()> {
  let globals = lua.globals();
  for name in DISABLED_GLOBALS {
    globals
      .set(*name, LuaValue::Nil)
      .map_err(|e| Error::Command(format!("ERR sandbox setup failed: {e}")))?;
  }

  let string_table: mlua::Table = globals
    .get("string")
    .map_err(|e| Error::Command(format!("ERR sandbox setup failed: {e}")))?;
  string_table
    .set("dump", LuaValue::Nil)
    .map_err(|e| Error::Command(format!("ERR sandbox setup failed: {e}")))?;

  let math_table: mlua::Table = globals
    .get("math")
    .map_err(|e| Error::Command(format!("ERR sandbox setup failed: {e}")))?;
  math_table
    .set(
      "randomseed",
      lua
        .create_function(|_, _: mlua::MultiValue| Ok(()))
        .map_err(|e| Error::Command(format!("ERR sandbox setup failed: {e}")))?,
    )
    .map_err(|e| Error::Command(format!("ERR sandbox setup failed: {e}")))?;

  Ok(())
}
