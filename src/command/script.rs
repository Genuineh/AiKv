use crate::error::{AikvError, Result};
use crate::protocol::RespValue;
use crate::storage::StorageAdapter;
use bytes::Bytes;
use mlua::{Lua, LuaOptions, StdLib, Value as LuaValue};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Script cache entry
#[derive(Clone, Debug)]
struct CachedScript {
    script: String,
}

/// Script command handler
pub struct ScriptCommands {
    storage: StorageAdapter,
    script_cache: Arc<RwLock<HashMap<String, CachedScript>>>,
}

impl ScriptCommands {
    pub fn new(storage: StorageAdapter) -> Self {
        Self {
            storage,
            script_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Calculate SHA1 hash of a script
    fn calculate_sha1(script: &str) -> String {
        let mut hasher = Sha1::new();
        hasher.update(script.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// EVAL script numkeys [key [key ...]] [arg [arg ...]]
    /// Execute a Lua script
    pub fn eval(&self, args: &[Bytes], db_index: usize) -> Result<RespValue> {
        if args.len() < 2 {
            return Err(AikvError::WrongArgCount("EVAL".to_string()));
        }

        let script = String::from_utf8_lossy(&args[0]).to_string();
        let numkeys: usize = String::from_utf8_lossy(&args[1])
            .parse()
            .map_err(|_| AikvError::InvalidArgument("numkeys must be a number".to_string()))?;

        if args.len() < 2 + numkeys {
            return Err(AikvError::InvalidArgument(
                "Number of keys doesn't match numkeys parameter".to_string(),
            ));
        }

        let keys: Vec<String> = args[2..2 + numkeys]
            .iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect();

        let argv: Vec<String> = args[2 + numkeys..]
            .iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect();

        self.execute_script(&script, &keys, &argv, db_index)
    }

    /// EVALSHA sha1 numkeys [key [key ...]] [arg [arg ...]]
    /// Execute a cached script by its SHA1 digest
    pub fn evalsha(&self, args: &[Bytes], db_index: usize) -> Result<RespValue> {
        if args.len() < 2 {
            return Err(AikvError::WrongArgCount("EVALSHA".to_string()));
        }

        let sha1 = String::from_utf8_lossy(&args[0]).to_string();
        let numkeys: usize = String::from_utf8_lossy(&args[1])
            .parse()
            .map_err(|_| AikvError::InvalidArgument("numkeys must be a number".to_string()))?;

        if args.len() < 2 + numkeys {
            return Err(AikvError::InvalidArgument(
                "Number of keys doesn't match numkeys parameter".to_string(),
            ));
        }

        // Get script from cache
        let cache = self
            .script_cache
            .read()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        let cached_script = cache.get(&sha1).ok_or_else(|| {
            AikvError::InvalidArgument("NOSCRIPT No matching script. Use EVAL.".to_string())
        })?;

        let script = cached_script.script.clone();
        drop(cache);

        let keys: Vec<String> = args[2..2 + numkeys]
            .iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect();

        let argv: Vec<String> = args[2 + numkeys..]
            .iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect();

        self.execute_script(&script, &keys, &argv, db_index)
    }

    /// SCRIPT LOAD script
    /// Load a script into the cache without executing it
    pub fn script_load(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.len() != 1 {
            return Err(AikvError::WrongArgCount("SCRIPT LOAD".to_string()));
        }

        let script = String::from_utf8_lossy(&args[0]).to_string();
        let sha1 = Self::calculate_sha1(&script);

        let mut cache = self
            .script_cache
            .write()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        cache.insert(
            sha1.clone(),
            CachedScript {
                script,
            },
        );

        Ok(RespValue::bulk_string(Bytes::from(sha1)))
    }

    /// SCRIPT EXISTS sha1 [sha1 ...]
    /// Check if scripts exist in the cache
    pub fn script_exists(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("SCRIPT EXISTS".to_string()));
        }

        let cache = self
            .script_cache
            .read()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        let results: Vec<RespValue> = args
            .iter()
            .map(|sha1_bytes| {
                let sha1 = String::from_utf8_lossy(sha1_bytes).to_string();
                let exists = cache.contains_key(&sha1);
                RespValue::Integer(if exists { 1 } else { 0 })
            })
            .collect();

        Ok(RespValue::Array(Some(results)))
    }

    /// SCRIPT FLUSH [ASYNC|SYNC]
    /// Clear the script cache
    pub fn script_flush(&self, _args: &[Bytes]) -> Result<RespValue> {
        let mut cache = self
            .script_cache
            .write()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        cache.clear();
        Ok(RespValue::simple_string("OK"))
    }

    /// SCRIPT KILL
    /// Kill the currently executing script (not implemented for now)
    pub fn script_kill(&self, _args: &[Bytes]) -> Result<RespValue> {
        // In a single-threaded execution model, this is not really applicable
        // Return NOTBUSY when no script is running
        Err(AikvError::InvalidArgument(
            "NOTBUSY No scripts in execution right now.".to_string(),
        ))
    }

    /// Execute a Lua script with given keys and arguments
    fn execute_script(
        &self,
        script: &str,
        keys: &[String],
        argv: &[String],
        db_index: usize,
    ) -> Result<RespValue> {
        // Create a new Lua instance with minimal standard library
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            LuaOptions::default(),
        )
        .map_err(|e| AikvError::Script(format!("Failed to create Lua instance: {}", e)))?;

        // Set up KEYS and ARGV tables
        lua.globals()
            .set("KEYS", lua.create_table().unwrap())
            .map_err(|e| AikvError::Script(format!("Failed to set KEYS: {}", e)))?;

        lua.globals()
            .set("ARGV", lua.create_table().unwrap())
            .map_err(|e| AikvError::Script(format!("Failed to set ARGV: {}", e)))?;

        // Populate KEYS (1-indexed in Lua)
        let keys_table = lua.globals().get::<mlua::Table>("KEYS").unwrap();
        for (i, key) in keys.iter().enumerate() {
            keys_table
                .set(i + 1, key.clone())
                .map_err(|e| AikvError::Script(format!("Failed to set KEYS[{}]: {}", i + 1, e)))?;
        }

        // Populate ARGV (1-indexed in Lua)
        let argv_table = lua.globals().get::<mlua::Table>("ARGV").unwrap();
        for (i, arg) in argv.iter().enumerate() {
            argv_table
                .set(i + 1, arg.clone())
                .map_err(|e| AikvError::Script(format!("Failed to set ARGV[{}]: {}", i + 1, e)))?;
        }

        // Set up redis.call and redis.pcall functions
        let storage = self.storage.clone();
        let db_index_for_call = db_index;

        lua.globals()
            .set(
                "redis",
                lua.create_table().map_err(|e| {
                    AikvError::Script(format!("Failed to create redis table: {}", e))
                })?,
            )
            .map_err(|e| AikvError::Script(format!("Failed to set redis table: {}", e)))?;

        let redis_table = lua.globals().get::<mlua::Table>("redis").unwrap();

        // redis.call - Execute Redis command (throws error on failure)
        let storage_for_call = storage.clone();
        let call_fn = lua
            .create_function(move |lua_ctx, args: mlua::MultiValue| {
                Self::redis_call(&storage_for_call, db_index_for_call, lua_ctx, args, true)
            })
            .map_err(|e| AikvError::Script(format!("Failed to create call function: {}", e)))?;

        redis_table
            .set("call", call_fn)
            .map_err(|e| AikvError::Script(format!("Failed to set redis.call: {}", e)))?;

        // redis.pcall - Protected call (returns error as result)
        let storage_for_pcall = storage.clone();
        let pcall_fn = lua
            .create_function(move |lua_ctx, args: mlua::MultiValue| {
                Self::redis_call(&storage_for_pcall, db_index_for_call, lua_ctx, args, false)
            })
            .map_err(|e| AikvError::Script(format!("Failed to create pcall function: {}", e)))?;

        redis_table
            .set("pcall", pcall_fn)
            .map_err(|e| AikvError::Script(format!("Failed to set redis.pcall: {}", e)))?;

        // Execute the script
        let result: LuaValue = lua
            .load(script)
            .eval()
            .map_err(|e| AikvError::Script(format!("Script execution error: {}", e)))?;

        // Convert Lua result to RespValue
        Self::lua_to_resp(result)
    }

    /// Execute a Redis command from Lua
    fn redis_call(
        storage: &StorageAdapter,
        db_index: usize,
        lua: &mlua::Lua,
        args: mlua::MultiValue,
        throw_error: bool,
    ) -> mlua::Result<LuaValue> {
        // Convert arguments to bytes
        let mut cmd_args: Vec<Bytes> = Vec::new();

        for arg in args {
            match arg {
                LuaValue::String(s) => {
                    cmd_args.push(Bytes::from(s.as_bytes().to_vec()));
                }
                LuaValue::Integer(i) => {
                    cmd_args.push(Bytes::from(i.to_string()));
                }
                LuaValue::Number(n) => {
                    cmd_args.push(Bytes::from(n.to_string()));
                }
                LuaValue::Boolean(b) => {
                    cmd_args.push(Bytes::from(if b { "1" } else { "0" }));
                }
                _ => {
                    if throw_error {
                        return Err(mlua::Error::RuntimeError(
                            "Invalid argument type".to_string(),
                        ));
                    } else {
                        return Ok(LuaValue::Nil);
                    }
                }
            }
        }

        if cmd_args.is_empty() {
            if throw_error {
                return Err(mlua::Error::RuntimeError(
                    "No command specified".to_string(),
                ));
            } else {
                return Ok(LuaValue::Nil);
            }
        }

        // Extract command and arguments
        let command = String::from_utf8_lossy(&cmd_args[0])
            .to_uppercase()
            .to_string();
        let command_args = &cmd_args[1..];

        // Execute simple string commands
        let result = match command.as_str() {
            "GET" => Self::execute_get(storage, command_args, db_index),
            "SET" => Self::execute_set(storage, command_args, db_index),
            "DEL" => Self::execute_del(storage, command_args, db_index),
            "EXISTS" => Self::execute_exists(storage, command_args, db_index),
            _ => {
                if throw_error {
                    return Err(mlua::Error::RuntimeError(format!(
                        "Command not supported in scripts: {}",
                        command
                    )));
                } else {
                    return Ok(LuaValue::Nil);
                }
            }
        };

        match result {
            Ok(resp_value) => Self::resp_to_lua(lua, resp_value),
            Err(e) => {
                if throw_error {
                    Err(mlua::Error::RuntimeError(format!(
                        "Command execution error: {}",
                        e
                    )))
                } else {
                    Ok(LuaValue::Nil)
                }
            }
        }
    }

    /// Execute GET command
    fn execute_get(storage: &StorageAdapter, args: &[Bytes], db_index: usize) -> Result<RespValue> {
        if args.len() != 1 {
            return Err(AikvError::WrongArgCount("GET".to_string()));
        }
        let key = String::from_utf8_lossy(&args[0]).to_string();
        match storage.get_from_db(db_index, &key)? {
            Some(value) => Ok(RespValue::bulk_string(value)),
            None => Ok(RespValue::Null),
        }
    }

    /// Execute SET command
    fn execute_set(storage: &StorageAdapter, args: &[Bytes], db_index: usize) -> Result<RespValue> {
        if args.len() < 2 {
            return Err(AikvError::WrongArgCount("SET".to_string()));
        }
        let key = String::from_utf8_lossy(&args[0]).to_string();
        let value = args[1].clone();
        storage.set_in_db(db_index, key, value)?;
        Ok(RespValue::simple_string("OK"))
    }

    /// Execute DEL command
    fn execute_del(storage: &StorageAdapter, args: &[Bytes], db_index: usize) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("DEL".to_string()));
        }
        let mut count = 0;
        for arg in args {
            let key = String::from_utf8_lossy(arg).to_string();
            if storage.delete_from_db(db_index, &key)? {
                count += 1;
            }
        }
        Ok(RespValue::Integer(count))
    }

    /// Execute EXISTS command
    fn execute_exists(
        storage: &StorageAdapter,
        args: &[Bytes],
        db_index: usize,
    ) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("EXISTS".to_string()));
        }
        let mut count = 0;
        for arg in args {
            let key = String::from_utf8_lossy(arg).to_string();
            if storage.exists_in_db(db_index, &key)? {
                count += 1;
            }
        }
        Ok(RespValue::Integer(count))
    }

    /// Convert Lua value to RESP value
    fn lua_to_resp(value: LuaValue) -> Result<RespValue> {
        match value {
            LuaValue::Nil => Ok(RespValue::Null),
            LuaValue::Boolean(b) => Ok(RespValue::Integer(if b { 1 } else { 0 })),
            LuaValue::Integer(i) => Ok(RespValue::Integer(i)),
            LuaValue::Number(n) => {
                // Convert float to integer if possible, otherwise to string
                if n.fract() == 0.0 {
                    Ok(RespValue::Integer(n as i64))
                } else {
                    Ok(RespValue::bulk_string(Bytes::from(n.to_string())))
                }
            }
            LuaValue::String(s) => Ok(RespValue::bulk_string(Bytes::from(s.as_bytes().to_vec()))),
            LuaValue::Table(t) => {
                // Convert table to array
                let mut results = Vec::new();
                for i in 1..=t.len().unwrap_or(0) {
                    if let Ok(val) = t.get::<LuaValue>(i) {
                        results.push(Self::lua_to_resp(val)?);
                    }
                }
                Ok(RespValue::Array(Some(results)))
            }
            _ => Ok(RespValue::Null),
        }
    }

    /// Convert RESP value to Lua value
    fn resp_to_lua(lua: &mlua::Lua, value: RespValue) -> mlua::Result<LuaValue> {
        match value {
            RespValue::Null => Ok(LuaValue::Boolean(false)),
            RespValue::SimpleString(s) => {
                Ok(LuaValue::String(lua.create_string(s.as_bytes()).map_err(
                    |e| mlua::Error::RuntimeError(format!("Failed to create string: {}", e)),
                )?))
            }
            RespValue::Error(e) => Ok(LuaValue::String(lua.create_string(e.as_bytes()).map_err(
                |e| mlua::Error::RuntimeError(format!("Failed to create error string: {}", e)),
            )?)),
            RespValue::Integer(i) => Ok(LuaValue::Integer(i)),
            RespValue::BulkString(opt_b) => match opt_b {
                Some(b) => Ok(LuaValue::String(lua.create_string(&b).map_err(|e| {
                    mlua::Error::RuntimeError(format!("Failed to create bulk string: {}", e))
                })?)),
                None => Ok(LuaValue::Boolean(false)),
            },
            RespValue::Array(opt_arr) => match opt_arr {
                Some(arr) => {
                    let table = lua.create_table().map_err(|e| {
                        mlua::Error::RuntimeError(format!("Failed to create table: {}", e))
                    })?;
                    for (i, item) in arr.into_iter().enumerate() {
                        table
                            .set(i + 1, Self::resp_to_lua(lua, item)?)
                            .map_err(|e| {
                                mlua::Error::RuntimeError(format!(
                                    "Failed to set table item: {}",
                                    e
                                ))
                            })?;
                    }
                    Ok(LuaValue::Table(table))
                }
                None => Ok(LuaValue::Boolean(false)),
            },
            _ => Ok(LuaValue::Nil),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageAdapter;

    fn setup() -> ScriptCommands {
        let storage = StorageAdapter::with_db_count(16);
        ScriptCommands::new(storage)
    }

    #[test]
    fn test_calculate_sha1() {
        let script = "return 1";
        let sha1 = ScriptCommands::calculate_sha1(script);
        assert_eq!(sha1, "e0e1f9fabfc9d4800c877a703b823ac0578ff8db");
    }

    #[test]
    fn test_script_load() {
        let script_commands = setup();
        let script = "return 'hello'";
        let args = vec![Bytes::from(script)];

        let result = script_commands.script_load(&args);
        assert!(result.is_ok());

        if let Ok(RespValue::BulkString(Some(sha1))) = result {
            assert_eq!(
                String::from_utf8_lossy(&sha1),
                "1b936e3fe509bcbc9cd0664897bbe8fd0cac101b"
            );
        } else {
            panic!("Expected BulkString");
        }
    }

    #[test]
    fn test_script_exists() {
        let script_commands = setup();
        let script = "return 'hello'";
        let sha1 = ScriptCommands::calculate_sha1(script);

        // Load the script first
        let args = vec![Bytes::from(script)];
        script_commands.script_load(&args).unwrap();

        // Check if script exists
        let exists_args = vec![Bytes::from(sha1)];
        let result = script_commands.script_exists(&exists_args).unwrap();

        if let RespValue::Array(Some(arr)) = result {
            assert_eq!(arr.len(), 1);
            assert_eq!(arr[0], RespValue::Integer(1));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_script_flush() {
        let script_commands = setup();
        let script = "return 'hello'";

        // Load a script
        let args = vec![Bytes::from(script)];
        script_commands.script_load(&args).unwrap();

        // Flush the cache
        let result = script_commands.script_flush(&[]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), RespValue::simple_string("OK"));

        // Check that script no longer exists
        let sha1 = ScriptCommands::calculate_sha1(script);
        let exists_args = vec![Bytes::from(sha1)];
        let result = script_commands.script_exists(&exists_args).unwrap();

        if let RespValue::Array(Some(arr)) = result {
            assert_eq!(arr[0], RespValue::Integer(0));
        }
    }

    #[test]
    fn test_eval_simple_return() {
        let script_commands = setup();
        let script = "return 42";
        let args = vec![Bytes::from(script), Bytes::from("0")];

        let result = script_commands.eval(&args, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), RespValue::Integer(42));
    }

    #[test]
    fn test_eval_with_keys() {
        let script_commands = setup();
        let script = "return KEYS[1]";
        let args = vec![Bytes::from(script), Bytes::from("1"), Bytes::from("mykey")];

        let result = script_commands.eval(&args, 0).unwrap();
        if let RespValue::BulkString(Some(value)) = result {
            assert_eq!(String::from_utf8_lossy(&value), "mykey");
        } else {
            panic!("Expected BulkString");
        }
    }

    #[test]
    fn test_eval_with_argv() {
        let script_commands = setup();
        let script = "return ARGV[1]";
        let args = vec![Bytes::from(script), Bytes::from("0"), Bytes::from("myarg")];

        let result = script_commands.eval(&args, 0).unwrap();
        if let RespValue::BulkString(Some(value)) = result {
            assert_eq!(String::from_utf8_lossy(&value), "myarg");
        } else {
            panic!("Expected BulkString");
        }
    }

    #[test]
    fn test_eval_redis_call_set_get() {
        let script_commands = setup();
        let script = r#"
            redis.call('SET', KEYS[1], ARGV[1])
            return redis.call('GET', KEYS[1])
        "#;
        let args = vec![
            Bytes::from(script),
            Bytes::from("1"),
            Bytes::from("mykey"),
            Bytes::from("myvalue"),
        ];

        let result = script_commands.eval(&args, 0).unwrap();
        if let RespValue::BulkString(Some(value)) = result {
            assert_eq!(String::from_utf8_lossy(&value), "myvalue");
        } else {
            panic!("Expected BulkString");
        }
    }

    #[test]
    fn test_evalsha() {
        let script_commands = setup();
        let script = "return 'hello from cache'";

        // Load the script first
        let load_args = vec![Bytes::from(script)];
        let load_result = script_commands.script_load(&load_args).unwrap();

        let sha1 = if let RespValue::BulkString(Some(sha)) = load_result {
            sha
        } else {
            panic!("Expected BulkString");
        };

        // Execute using EVALSHA
        let evalsha_args = vec![sha1, Bytes::from("0")];
        let result = script_commands.evalsha(&evalsha_args, 0).unwrap();

        if let RespValue::BulkString(Some(value)) = result {
            assert_eq!(String::from_utf8_lossy(&value), "hello from cache");
        } else {
            panic!("Expected BulkString");
        }
    }

    #[test]
    fn test_evalsha_not_found() {
        let script_commands = setup();
        let sha1 = "nonexistent_sha1";
        let args = vec![Bytes::from(sha1), Bytes::from("0")];

        let result = script_commands.evalsha(&args, 0);
        assert!(result.is_err());
    }
}
